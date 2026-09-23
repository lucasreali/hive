import { openModal } from "../store";
import { PlusIcon } from "./icons";

// Project tree arrives with 1.5 (projects) and 2.2 (agents); the pending counter with 2.3.
export function Sidebar() {
  return (
    <nav className="sidebar" aria-label="Projects">
      <div className="bar">
        <span className="sidebar-pending">Nothing pending</span>
        <button
          type="button"
          className="ghost"
          title="Add project (Ctrl+Shift+O)"
          onClick={() => openModal("add-project")}
        >
          <PlusIcon />
          <span>Project</span>
        </button>
      </div>
      <div className="sidebar-tree">
        <div className="hint">No projects</div>
      </div>
    </nav>
  );
}
