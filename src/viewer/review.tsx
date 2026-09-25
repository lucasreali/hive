import { CloseIcon } from "../shell/icons";
import { type HiveState, type ReviewComment, useHive } from "../store";
import { pasteToTerminal, showTerminal, terminal } from "../terminals";
import { keyText } from "../window";
import { reference, terminalIn } from "./reference";

// Review comments (6.7): notes on lines of the open file, kept per worktree, then sent to the
// worktree's terminal as one pasted text for the agent to read. Nothing is submitted: no Enter.

/**
 * Ctrl+Shift+M and the file view's "Comment": opens the comment input for the selected lines
 * of the shown file.
 */
export function startComment(): void {
  const s = useHive.getState();
  if (!s.fileShown || !s.openFile || !s.selectedLines) return;
  useHive.setState({ commenting: { ...s.openFile, ...s.selectedLines } });
}

/** Closes the comment input, keeping `text` (trimmed) as a comment unless it is empty. */
export function endComment(text = ""): void {
  useHive.setState((s) => {
    const at = s.commenting;
    const note = text.trim();
    if (!at || !note) return { commenting: null };
    const comment = { path: at.path, from: at.from, to: at.to, text: note };
    const kept = s.comments[at.worktree] ?? [];
    return { commenting: null, comments: { ...s.comments, [at.worktree]: [...kept, comment] } };
  });
}

/** Drops the comment at `index` of `worktree`'s review. */
export const deleteComment = (worktree: string, index: number) =>
  useHive.setState((s) => ({
    comments: {
      ...s.comments,
      [worktree]: (s.comments[worktree] ?? []).filter((_, i) => i !== index),
    },
  }));

/** One line per comment: its `@` reference, then `— ` and the comment. */
export const reviewText = (comments: ReviewComment[]) =>
  comments.map((c) => `${reference(c.path, c)}— ${c.text}`).join("\n");

/**
 * Where and what "Send review" pastes: the open file's worktree's comments, into the terminal
 * a reference would go to; or why it cannot.
 */
export function reviewTarget(
  s: HiveState,
): { terminal: number; worktree: string; text: string } | { why: string } {
  const comments = s.openFile ? (s.comments[s.openFile.worktree] ?? []) : [];
  if (!s.openFile || comments.length === 0) return { why: "No comments to send" };
  const target = terminalIn(s, s.openFile.worktree);
  if ("why" in target) return target;
  return { ...target, worktree: s.openFile.worktree, text: reviewText(comments) };
}

/**
 * Shows and focuses the terminal (in front of the file's tab: xterm pastes only into a
 * terminal it has shown), pastes the review into it and drops the sent comments.
 */
export function sendReview(): void {
  const target = reviewTarget(useHive.getState());
  if ("why" in target || !terminal(target.terminal)) return;
  useHive.setState((s) => ({
    fileShown: false,
    comments: { ...s.comments, [target.worktree]: [] },
  }));
  showTerminal(target.terminal);
  pasteToTerminal(target.terminal, target.text);
}

const lines = ({ from, to }: { from: number; to: number }) =>
  from === to ? `${from}` : `${from}–${to}`;

/** The comment input, under the file view's header while a comment is being written. */
export function CommentInput({ worktree, path }: { worktree: string; path: string }) {
  const at = useHive((s) => s.commenting);
  if (at?.worktree !== worktree || at.path !== path) return null;
  return (
    <div className="comment-input">
      <label htmlFor="comment-text">{`Line${at.from === at.to ? "" : "s"} ${lines(at)}`}</label>
      <input
        id="comment-text"
        className="comment-text"
        // biome-ignore lint/a11y/noAutofocus: the input opens on the user's own action
        autoFocus
        placeholder="Comment for the agent… (Enter saves, Esc cancels)"
        onKeyDown={(event) => {
          if (event.key === "Enter") endComment(event.currentTarget.value);
          else if (event.key === "Escape") endComment();
        }}
      />
    </div>
  );
}

/** The worktree's comments not sent yet, each with ×, and "Send review (N)". */
export function ReviewList({ worktree }: { worktree: string }) {
  const comments = useHive((s) => s.comments[worktree]);
  const unsendable = useHive((s) => {
    const target = reviewTarget(s);
    return "why" in target ? target.why : null;
  });
  if (!comments?.length) return null;
  return (
    <section className="review" aria-label="Review comments">
      <div className="review-bar">
        <span className="review-title">Review</span>
        <button
          type="button"
          className="ghost text"
          title={unsendable ?? "Paste the comments into the terminal, without Enter"}
          disabled={unsendable !== null}
          onClick={sendReview}
        >
          {`Send review (${comments.length})`}
        </button>
      </div>
      <ul>
        {comments.map((c, i) => (
          <li className="review-item" key={`${c.path}:${c.from}:${c.to}:${c.text}`}>
            <span className="review-where">{`${c.path}:${lines(c)}`}</span>
            <span className="review-text">{c.text}</span>
            <button
              type="button"
              className="ghost review-delete"
              aria-label="Delete comment"
              title="Delete comment"
              onClick={() => deleteComment(worktree, i)}
            >
              <CloseIcon />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The file view header's "Comment" button. */
export function CommentButton() {
  const why = useHive((s) => (s.selectedLines ? null : "Select lines to comment on them"));
  return (
    <button
      type="button"
      className="ghost text"
      title={why ?? keyText("Comment on the selected lines (Ctrl+Shift+M)")}
      disabled={why !== null}
      onClick={startComment}
    >
      Comment
    </button>
  );
}
