import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState } from "react";
import { openModal, select, useHive } from "../store";
import { transport } from "../transport";
import { BranchIcon, CheckIcon, CloseIcon } from "./icons";

const close = () => openModal(null);

type Row = { head: string } | { branch: string };

/**
 * Screens 1c/1d. Every rule is the service's (#33, #37): it validates the name as the user
 * types and again on create, lists the branches and creates the worktree with the CLI's code.
 * The dialog only filters the branch list and shows the answers.
 */
export function NewWorktreeDialog() {
  const projects = Object.values(useHive((s) => s.projects) ?? {});
  const initial = useHive((s) => s.modalProject);
  const { branches, nameChecks, created, createFailure } = useHive((s) => s.worktreeDialog);
  const [project, setProject] = useState(initial ?? projects[0]?.id ?? "");
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [openTerminal, setOpenTerminal] = useState(true);

  useEffect(() => void transport.listBranches(project), [project]);
  useEffect(() => void transport.validateWorktreeName(project, name), [project, name]);

  // Answers for another project or name (the user kept typing) are ignored.
  const list = branches?.project === project ? branches : null;
  const check = nameChecks[name]?.project === project ? nameChecks[name] : null;
  const failed = createFailure?.project === project && createFailure.name === name;
  const error = failed ? createFailure.message : name && check?.error;

  const q = query.toLowerCase();
  const match = (b: string) => b.toLowerCase().includes(q);
  const local = list?.local.filter(match) ?? [];
  const remote = list?.remote.filter(match) ?? [];
  const shown = [...local, ...remote];
  const wanted = picked ?? list?.current ?? list?.local[0] ?? null;
  // A branch hidden by the filter gives way to the first one shown, as in the prototype.
  const base =
    wanted && (shown.includes(wanted) || shown.length === 0) ? wanted : (shown[0] ?? null);
  const rows: Row[] = [
    ...(local.length ? [{ head: "Local" }, ...local.map((branch) => ({ branch }))] : []),
    ...(remote.length ? [{ head: "Remote" }, ...remote.map((branch) => ({ branch }))] : []),
  ];

  const scroller = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: (i) => ("head" in rows[i] ? 22 : 24),
    overscan: 6,
  });

  // The service created the worktree: open its terminal, select it, and close unless the
  // service had something to say about it.
  const handled = useRef(created);
  useEffect(() => {
    if (!created || handled.current === created) return;
    handled.current = created;
    if (openTerminal) void transport.openTerminal(created.path, 80, 24, () => {});
    select(created.path);
    if (created.notes.length === 0) close();
  }, [created, openTerminal]);

  const move = (step: number) => {
    const next = shown[Math.max(0, Math.min(shown.length - 1, shown.indexOf(base ?? "") + step))];
    if (next === undefined) return;
    setPicked(next);
    virtual.scrollToIndex(rows.findIndex((r) => "branch" in r && r.branch === next));
  };

  const canCreate = name !== "" && check?.error === null && !failed;

  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog"
      aria-labelledby="new-worktree-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          dialog.querySelector<HTMLInputElement>("input[name=name]")?.focus();
        }
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (created) close();
          else if (canCreate) void transport.createWorktree(project, name, base);
        }}
      >
        <header>
          <h2 id="new-worktree-title">New worktree</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        {created ? (
          <div className="dialog-body">
            <p className="created">
              Created <code>{created.path}</code>
            </p>
            <ul className="notes">
              {created.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </div>
        ) : (
          <div className="dialog-body wide">
            <label className="field">
              <span>Project</span>
              <select
                value={project}
                onChange={(e) => {
                  setProject(e.target.value);
                  setPicked(null);
                  setQuery("");
                }}
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="field">
              <label htmlFor="new-worktree-name">Worktree name</label>
              <input
                id="new-worktree-name"
                name="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. fix-cart"
                spellCheck={false}
                autoComplete="off"
                aria-invalid={Boolean(error)}
              />
              {error && (
                <span className="field-error" role="alert">
                  {error}
                </span>
              )}
            </div>
            <div className="field">
              <label htmlFor="new-worktree-filter">Base branch</label>
              <div className="branch-picker">
                <input
                  id="new-worktree-filter"
                  className="branch-filter"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
                    if (!step) return;
                    e.preventDefault();
                    move(step);
                  }}
                  placeholder="Filter local and remote branches"
                  spellCheck={false}
                  autoComplete="off"
                />
                <kbd>↑↓</kbd>
                <div className="branch-list" ref={scroller}>
                  {list && rows.length === 0 && (
                    <div className="branch-empty">{list.error ?? "No branches found"}</div>
                  )}
                  <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
                    {virtual.getVirtualItems().map((item) => {
                      const row = rows[item.index];
                      const style = { transform: `translateY(${item.start}px)` };
                      if ("head" in row) {
                        return (
                          <div key={row.head} className="branch-head" style={style}>
                            {row.head}
                          </div>
                        );
                      }
                      return (
                        // Keyboard users move with ↑↓ in the filter, so rows stay out of the tab order.
                        <button
                          type="button"
                          tabIndex={-1}
                          key={row.branch}
                          className="branch-row"
                          aria-pressed={row.branch === base}
                          style={style}
                          onClick={() => setPicked(row.branch)}
                        >
                          <BranchIcon />
                          <span className="branch-name">{row.branch}</span>
                          {row.branch === list?.current && <span className="badge">default</span>}
                          {row.branch === base && <CheckIcon />}
                        </button>
                      );
                    })}
                  </div>
                </div>
              </div>
            </div>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={openTerminal}
                onChange={(e) => setOpenTerminal(e.target.checked)}
              />
              Open a terminal in the new worktree
            </label>
            <div className="plan">
              <div>
                <span className="plan-label">Folder: </span>
                {check?.folder}
              </div>
              <div>
                <span className="plan-label">Branch: </span>
                {check?.branch}
                {base && <span className="plan-label"> (from {base})</span>}
              </div>
            </div>
          </div>
        )}
        <footer>
          {created ? (
            <button type="submit" className="primary">
              Close <kbd>Enter</kbd>
            </button>
          ) : (
            <>
              <button type="button" className="secondary" onClick={close}>
                Cancel <kbd>Esc</kbd>
              </button>
              <button type="submit" className="primary" disabled={!canCreate}>
                Create worktree <kbd>Enter</kbd>
              </button>
            </>
          )}
        </footer>
      </form>
    </dialog>
  );
}
