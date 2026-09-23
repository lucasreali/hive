import { useState } from "react";
import { openModal, useHive, type Worktree } from "../store";
import { openTerminal } from "../terminals";
import { BranchIcon, SearchIcon } from "./icons";

const close = () => openModal(null);

/**
 * Ctrl+Shift+T: every worktree of every project, filtered by worktree or project name;
 * arrows move, Enter (or a click) opens a terminal in the worktree, Esc closes.
 */
export function WorktreePicker() {
  const projects = useHive((s) => s.projects);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const q = query.toLowerCase();
  const items = Object.values(projects ?? {}).flatMap((project) =>
    project.worktrees
      .filter((w) => `${w.name} ${project.name}`.toLowerCase().includes(q))
      .map((worktree) => ({ worktree, project })),
  );
  const pick = (worktree: Worktree | undefined) => {
    if (!worktree) return;
    close();
    void openTerminal(worktree.path);
  };
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog picker"
      aria-label="Open a terminal in worktree"
      ref={(dialog) => {
        if (dialog && !dialog.open) dialog.showModal();
      }}
      onClose={close}
      // A click outside the picker lands on the dialog itself (its backdrop): it closes.
      onClick={(e) => e.target === e.currentTarget && close()}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") setIndex(Math.min(items.length - 1, index + 1));
        else if (e.key === "ArrowUp") setIndex(Math.max(0, index - 1));
        else if (e.key === "Enter") pick(items[index]?.worktree);
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
          placeholder="Open a terminal in worktree…"
          spellCheck={false}
        />
      </label>
      <div className="picker-list">
        {items.map(({ worktree, project }, i) => (
          <button
            key={worktree.id}
            type="button"
            className="picker-row"
            aria-pressed={i === index}
            ref={i === index ? (row) => row?.scrollIntoView({ block: "nearest" }) : undefined}
            onMouseEnter={() => setIndex(i)}
            onClick={() => pick(worktree)}
          >
            <BranchIcon />
            <span className="picker-name">{worktree.name}</span>
            <span className="picker-project">{project.name}</span>
            <span className="picker-path">{worktree.path}</span>
          </button>
        ))}
        {items.length === 0 && <div className="branch-empty">No worktrees found</div>}
      </div>
      <footer className="picker-hints">
        <span>
          <kbd>↑↓</kbd>navigate
        </span>
        <span>
          <kbd>Enter</kbd>open terminal
        </span>
        <span>
          <kbd>Esc</kbd>close
        </span>
      </footer>
    </dialog>
  );
}
