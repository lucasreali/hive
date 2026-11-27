import { useEffect, useRef, useState } from "react";
import { askCommit, openPush, stageFiles } from "../git";
import type { Worktree } from "../protocol";
import { owner, useHive } from "../store";

/**
 * The Changes panel's commit box (27.6, D27.3): how many of the files left to commit are staged,
 * with a box staging or unstaging them all, the commit message, "Commit" (asked first) and
 * "Push…" (its dialog). Each file's own box is on its row of the tree. The service runs git: a
 * failure shows here as git said it; the files watch brings the new changes. In a jj repository's
 * main worktree a note says jj owns the index there (28.26).
 */
export function CommitBox({ worktree }: { worktree: Worktree }) {
  const path = worktree.path;
  const changes = useHive((s) => s.changes[path]);
  const git = useHive((s) => s.git[path]);
  const project = useHive((s) => owner(s.projects, worktree.id)?.id);
  // jj resets git's index and moves its HEAD (28.26): only the main worktree is jj's.
  const jj = useHive((s) => worktree.main && owner(s.projects, worktree.id)?.jj === true);
  const [message, setMessage] = useState("");
  const busy = git?.busy ?? null;
  // A commit that went through empties the message.
  const was = useRef(busy);
  useEffect(() => {
    if (was.current === "commit" && busy === null && !git?.error) setMessage("");
    was.current = busy;
  }, [busy, git?.error]);
  if (!changes) return null;
  const open = changes.files.filter((f) => f.staged !== null);
  const staged = open.filter((f) => f.staged !== "no");
  const all = open.length > 0 && open.every((f) => f.staged === "yes");
  const error = git?.error && git.error.action !== "push" ? git.error.message : null;
  return (
    <section className="commit-box" aria-label="Commit">
      {jj && (
        <p className="commit-box-note" role="note">
          This is a jj repository: jj rewrites git's index; commit with jj
        </p>
      )}
      <div className="commit-box-bar">
        <label className="checkbox">
          <input
            type="checkbox"
            checked={all}
            disabled={open.length === 0 || busy !== null}
            ref={(box) => {
              if (box) box.indeterminate = staged.length > 0 && !all;
            }}
            onChange={() =>
              stageFiles(
                path,
                open.map((f) => f.path),
                !all,
              )
            }
          />
          {`${staged.length} of ${open.length} staged`}
        </label>
      </div>
      <textarea
        aria-label="Commit message"
        placeholder="Commit message"
        value={message}
        rows={3}
        onChange={(e) => setMessage(e.target.value)}
      />
      <div className="commit-box-actions">
        <button
          type="button"
          className="primary"
          disabled={busy !== null || staged.length === 0 || !message.trim()}
          onClick={() => askCommit(path, worktree.branch, staged.length, message)}
        >
          {busy === "commit" ? "Committing…" : "Commit"}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy !== null || !worktree.branch || !project}
          title={worktree.branch ? undefined : "This worktree is on no branch"}
          onClick={() => openPush(project as string, worktree)}
        >
          Push…
        </button>
      </div>
      {error && (
        <pre className="field-error git-output" role="alert">
          {error}
        </pre>
      )}
    </section>
  );
}
