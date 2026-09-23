import { openModal, setRightPanel, useHive } from "../store";
import { AddFolderIcon, PanelIcon, PlusIcon } from "./icons";

/** Screen 1e: shown once the service said there are no projects. */
function EmptyState() {
  return (
    <div className="empty-state">
      <div className="empty-state-content">
        <AddFolderIcon />
        <div>
          <h2>No project open</h2>
          <p>Add a project to follow the agents running in its worktrees.</p>
        </div>
        <button type="button" className="primary" onClick={() => openModal("add-project")}>
          Add project <kbd>Ctrl+Shift+O</kbd>
        </button>
        <div className="empty-state-example">
          <span>A project is a folder inside WSL, for example:</span>
          <code>/home/user/projects/shop</code>
        </div>
      </div>
    </div>
  );
}

// Tabs and xterm.js terminals arrive with 1.7; the worktree picker behind "+" with 1.9.
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
  const empty = useHive((s) => s.projects !== null && Object.keys(s.projects).length === 0);
  return (
    <section className="terminals" aria-label="Terminals">
      <div className="bar">
        <div className="tabs" role="tablist">
          <button type="button" className="ghost" title="New terminal (Ctrl+Shift+T)" disabled>
            <PlusIcon size={14} />
          </button>
        </div>
        <div className="tabs-actions">
          <button
            type="button"
            className="ghost"
            title="Files and diff (Ctrl+Shift+B)"
            aria-pressed={open}
            onClick={() => setRightPanel(open ? null : "files")}
          >
            <PanelIcon />
          </button>
        </div>
      </div>
      <div className="terminal-body">{empty && <EmptyState />}</div>
    </section>
  );
}
