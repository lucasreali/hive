import { useRef } from "react";
import type { ReviewComment } from "../protocol";
import { ago } from "../sessions";
import { CloseIcon } from "../shell/icons";
import { useKeptScroll, viewKey } from "../shell/panelPlace";
import { type HiveState, panelWorktree, setOpenFile, useHive } from "../store";
import { pasteToTerminal, showTerminal, terminal } from "../terminals";
import { transport } from "../transport";
import { composing, keyText } from "../window";
import { reference, terminalIn } from "./reference";

// Review comments (6.7, 27.3): notes on lines of a worktree's files, kept by the service, which
// moves each one with its code as the file changes (outdated once the code is gone). Sent to
// the worktree's terminal as pasted text for the agent to read. Nothing is submitted: no Enter.

/**
 * Ctrl+Shift+M and the file view's "Comment": opens the comment input for the selected lines
 * of the shown file.
 */
export function startComment(): void {
  const s = useHive.getState();
  if (!s.fileShown || !s.openFile || !s.selectedLines) return;
  useHive.setState({ commenting: { ...s.openFile, ...s.selectedLines } });
}

/** Closes the comment input, sending `text` (trimmed) to the service unless it is empty. */
export function endComment(text = ""): void {
  const at = useHive.getState().commenting;
  const note = text.trim();
  useHive.setState({ commenting: null });
  if (at && note) void transport.addComment(at.worktree, at.path, at.from, at.to, note);
}

/** A comment the agent can act on: open, with its code still there. */
export const sendable = (c: ReviewComment) => !c.resolved && !c.outdated;

/** One line per comment: its `@` reference, then `— ` and the comment. */
export const reviewText = (comments: ReviewComment[]) =>
  comments.map((c) => `${reference(c.file, c)}— ${c.body}`).join("\n");

/**
 * Where and what sending `comments` of `worktree` pastes: into the terminal a reference would go
 * to; or why it cannot.
 */
export function commentsTarget(
  s: HiveState,
  worktree: string,
  comments: ReviewComment[],
): { terminal: number; text: string } | { why: string } {
  if (comments.length === 0) return { why: "No open comments to send" };
  const target = terminalIn(s, worktree);
  if ("why" in target) return target;
  return { ...target, text: reviewText(comments) };
}

/** The shown worktree's open comments whose code is still there (the palette's command). */
export function openComments(s: HiveState): { worktree: string; comments: ReviewComment[] } {
  const worktree = panelWorktree(s)?.worktree.path ?? "";
  return { worktree, comments: (s.comments[worktree] ?? []).filter(sendable) };
}

/**
 * Shows and focuses the terminal (in front of a file's tab: xterm pastes only into a terminal it
 * has shown) and pastes `comments` of `worktree` into it. They stay until resolved.
 */
export function sendComments(worktree: string, comments: ReviewComment[]): void {
  const target = commentsTarget(useHive.getState(), worktree, comments);
  if ("why" in target || !terminal(target.terminal)) return;
  useHive.setState({ fileShown: false });
  showTerminal(target.terminal);
  pasteToTerminal(target.terminal, target.text);
}

/** The palette's "Send open comments". */
export function sendOpenComments(): void {
  const { worktree, comments } = openComments(useHive.getState());
  sendComments(worktree, comments);
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
          if (composing(event.nativeEvent)) return;
          if (event.key === "Enter") endComment(event.currentTarget.value);
          else if (event.key === "Escape") endComment();
        }}
      />
    </div>
  );
}

/** Why `comments` of `worktree` cannot be sent now, or null. */
const useUnsendable = (worktree: string, comments: ReviewComment[]) =>
  useHive((s) => {
    const target = commentsTarget(s, worktree, comments);
    return "why" in target ? target.why : null;
  });

/** One comment: where it is (a click opens the file there), its code, text and actions. */
function CommentItem({ worktree, comment: c }: { worktree: string; comment: ReviewComment }) {
  const unsendable = useUnsendable(worktree, [c]);
  return (
    <li className="comment-item" data-outdated={c.outdated || undefined}>
      <div className="comment-head">
        <button
          type="button"
          className="comment-where"
          title={c.outdated ? "Its code is gone: where it was" : "Open the file at these lines"}
          onClick={() => setOpenFile({ worktree, path: c.file }, false, c.from)}
        >
          {`${c.file}:${lines(c)}`}
        </button>
        <span className="comment-meta">{`${c.author} · ${ago(c.ms)}`}</span>
      </div>
      {c.code.length > 0 && <pre className="comment-code">{c.code.join("\n")}</pre>}
      <p className="comment-body">{c.body}</p>
      <div className="comment-actions">
        {sendable(c) && (
          <button
            type="button"
            className="ghost text"
            title={unsendable ?? "Paste the comment into the terminal, without Enter"}
            disabled={unsendable !== null}
            onClick={() => sendComments(worktree, [c])}
          >
            Send
          </button>
        )}
        <button
          type="button"
          className="ghost text"
          onClick={() => void transport.resolveComment(worktree, c.id, !c.resolved)}
        >
          {c.resolved ? "Reopen" : "Resolve"}
        </button>
        <button
          type="button"
          className="ghost comment-delete"
          aria-label="Delete comment"
          title="Delete comment"
          onClick={() => void transport.deleteComment(worktree, c.id)}
        >
          <CloseIcon />
        </button>
      </div>
    </li>
  );
}

/** A titled group of comments, left out when empty. */
function Group(props: { title: string; worktree: string; comments: ReviewComment[] }) {
  if (props.comments.length === 0) return null;
  return (
    <section className="comments-group" aria-label={props.title}>
      <h3 className="comments-title">{`${props.title} (${props.comments.length})`}</h3>
      <ul>
        {props.comments.map((c) => (
          <CommentItem key={c.id} worktree={props.worktree} comment={c} />
        ))}
      </ul>
    </section>
  );
}

const NONE: ReviewComment[] = [];

/**
 * The right panel's Comments view (27.3): the worktree's comments under Open, Outdated and
 * Resolved, and "Send all open".
 */
export function CommentsView({ worktree }: { worktree: string }) {
  const comments = useHive((s) => s.comments[worktree]) ?? NONE;
  const open = comments.filter(sendable);
  const unsendable = useUnsendable(worktree, open);
  // Its scroll, as this worktree's comments were left (28.19).
  const host = useHive((s) => s.host);
  const scroller = useRef<HTMLDivElement>(null);
  const onScroll = useKeptScroll(
    viewKey("comments", host, worktree),
    scroller,
    comments.length > 0,
  );
  if (comments.length === 0) {
    return (
      <div className="comments">
        <div className="hint">
          {keyText("No comments. Select lines of a file and press Comment (Ctrl+Shift+M).")}
        </div>
      </div>
    );
  }
  return (
    <div className="comments" ref={scroller} onScroll={onScroll}>
      <div className="comments-bar">
        <button
          type="button"
          className="ghost text"
          title={unsendable ?? "Paste the open comments into the terminal, without Enter"}
          disabled={unsendable !== null}
          onClick={() => sendComments(worktree, open)}
        >
          {`Send all open (${open.length})`}
        </button>
      </div>
      <Group title="Open" worktree={worktree} comments={open} />
      <Group
        title="Outdated"
        worktree={worktree}
        comments={comments.filter((c) => c.outdated && !c.resolved)}
      />
      <Group title="Resolved" worktree={worktree} comments={comments.filter((c) => c.resolved)} />
    </div>
  );
}

/** The file view header's "Comment" button. */
export function CommentButton({ live = true }: { live?: boolean }) {
  const why = useHive((s) => (live && s.selectedLines ? null : "Select lines to comment on them"));
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
