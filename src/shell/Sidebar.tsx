import { openModal, type Project, select, toggleCollapsed, useHive } from "../store";
import { transport } from "../transport";
import { BranchIcon, ChevronIcon, FolderIcon, PlusIcon, RefreshIcon } from "./icons";

// Agents join the tree in 1.8, states in 2.2, the pending counter in 2.3; arrow-key moves
// in the tree with 1.9 (#35). ponytail: plain list, add TanStack Virtual when trees get long.
export function Sidebar() {
  const projects = useHive((s) => s.projects);
  const list = Object.values(projects ?? {});
  return (
    <nav className="sidebar" aria-label="Projects">
      <div className="bar">
        <span className="sidebar-pending">Nothing pending</span>
        <div className="sidebar-actions">
          <button
            type="button"
            className="ghost icon"
            title="Refresh worktrees"
            onClick={() => void transport.listProjects()}
          >
            <RefreshIcon />
          </button>
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
      </div>
      <div className="sidebar-tree">
        {list.length === 0 ? (
          <div className="hint">No projects</div>
        ) : (
          <ul>
            {list.map((p) => (
              <ProjectNode key={p.id} project={p} />
            ))}
          </ul>
        )}
      </div>
    </nav>
  );
}

function ProjectNode({ project }: { project: Project }) {
  const open = useHive((s) => !s.collapsed[project.id]);
  const selection = useHive((s) => s.selection);
  return (
    <li>
      <div
        className="tree-row project"
        title={project.path}
        data-selected={selection === project.id}
      >
        <button
          type="button"
          className="chevron"
          aria-label={`${open ? "Collapse" : "Expand"} ${project.name}`}
          aria-expanded={open}
          onClick={() => toggleCollapsed(project.id)}
        >
          <ChevronIcon open={open} />
        </button>
        <button
          type="button"
          className="row-main"
          aria-current={selection === project.id}
          onClick={() => select(project.id)}
        >
          <FolderIcon />
          <span className="label">{project.name}</span>
        </button>
        <button
          type="button"
          className="new-worktree"
          title="New worktree (Ctrl+Shift+N)"
          onClick={() => openModal("new-worktree", project.id)}
        >
          <PlusIcon size={10} />
          New worktree
        </button>
      </div>
      {open && (
        <ul>
          {project.error && <li className="tree-error">{project.error}</li>}
          {project.worktrees.map((w) => (
            <li key={w.id}>
              <div className="tree-row worktree" title={w.path} data-selected={selection === w.id}>
                <span className="chevron" />
                <button
                  type="button"
                  className="row-main"
                  aria-current={selection === w.id}
                  onClick={() => select(w.id)}
                >
                  <BranchIcon />
                  <span className="label">{w.name}</span>
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}
