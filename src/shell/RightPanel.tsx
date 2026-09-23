import { setRightPanel } from "../store";
import { CloseIcon } from "./icons";

// Files and diff content arrives with Stage 3 (screen 1g).
export function RightPanel() {
  return (
    <aside className="right-panel" aria-label="Files and diff">
      <div className="bar">
        <span>Files and diff</span>
        <button
          type="button"
          className="ghost"
          title="Collapse (Ctrl+Shift+B)"
          onClick={() => setRightPanel(null)}
        >
          <CloseIcon />
        </button>
      </div>
      <div className="right-panel-empty">Select a project or agent to see its files.</div>
    </aside>
  );
}
