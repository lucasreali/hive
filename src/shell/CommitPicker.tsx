import { useEffect, useState } from "react";
import type { CommitInfo } from "../protocol";
import { setDiffBase, useHive } from "../store";
import { transport } from "../transport";
import { composing } from "../window";
import { SearchIcon } from "./icons";

/** The date a commit was made, as `YYYY-MM-DD`. */
const day = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/**
 * "A commit…" in the Changes panel's base menu (27.2): the worktree's last commits, as the
 * service lists them (`list_commits`, asked when it opens), its branch's first under "This
 * branch", filtered by subject, short id or author; arrows move, Enter (or a click) compares the
 * worktree with that commit, Esc closes.
 */
export function CommitPicker(props: { path: string; onClose: () => void }) {
  const { path, onClose } = props;
  const listed = useHive((s) => s.commits[path]);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  useEffect(() => {
    void transport.listCommits(path);
  }, [path]);
  const q = query.toLowerCase();
  const commits = (listed?.commits ?? []).filter((c) =>
    `${c.subject} ${c.short} ${c.author}`.toLowerCase().includes(q),
  );
  const pick = (commit: CommitInfo | undefined) => {
    if (!commit) return;
    onClose();
    setDiffBase(path, { commit: { id: commit.id } });
  };
  const empty = !listed ? "Loading commits…" : (listed.error ?? "No commits found");
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog picker"
      aria-label="Compare with a commit"
      ref={(dialog) => {
        if (dialog && !dialog.open) dialog.showModal();
      }}
      onClose={onClose}
      // A click outside the picker lands on the dialog itself (its backdrop): it closes.
      onClick={(e) => e.target === e.currentTarget && onClose()}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") setIndex(Math.min(commits.length - 1, index + 1));
        else if (e.key === "ArrowUp") setIndex(Math.max(0, index - 1));
        else if (e.key === "Enter" && !composing(e.nativeEvent)) pick(commits[index]);
        else return;
        e.preventDefault();
      }}
    >
      <label className="picker-search">
        <SearchIcon />
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          placeholder="Compare with a commit…"
          aria-label="Filter commits"
          spellCheck={false}
        />
      </label>
      <div className="picker-list">
        {commits.map((commit, i) => {
          const heading =
            commit.on_branch !== commits[i - 1]?.on_branch &&
            (commit.on_branch ? "This branch" : "Older commits");
          return (
            <div key={commit.id} className="commit-entry">
              {heading && <div className="picker-heading">{heading}</div>}
              <button
                type="button"
                className="picker-row commit-row"
                aria-pressed={i === index}
                ref={i === index ? (row) => row?.scrollIntoView({ block: "nearest" }) : undefined}
                onMouseEnter={() => setIndex(i)}
                onClick={() => pick(commit)}
              >
                <span className="picker-project commit-short">{commit.short}</span>
                <span className="picker-name">{commit.subject}</span>
                <span className="picker-path">
                  {commit.author} · {day(commit.time_ms)}
                </span>
              </button>
            </div>
          );
        })}
        {commits.length === 0 && <div className="branch-empty">{empty}</div>}
      </div>
      <footer className="picker-hints">
        <span>
          <kbd>↑↓</kbd>navigate
        </span>
        {listed?.truncated && <span>Only the last {listed.commits.length} commits</span>}
      </footer>
    </dialog>
  );
}
