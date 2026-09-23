import { setRightPanel, useHive } from "../store";
import { PanelIcon, PlusIcon } from "./icons";

// Tabs and xterm.js terminals arrive with 1.7; the worktree picker behind "+" with 1.9;
// the empty state (screen 1e) with 1.5.
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
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
      <div className="terminal-body" />
    </section>
  );
}
