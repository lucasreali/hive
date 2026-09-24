import type { KeyboardEvent, ReactNode } from "react";
import {
  type Agent,
  type AgentState,
  activateTab,
  openModal,
  type Project,
  select,
  toggleCollapsed,
  useHive,
} from "../store";
import { transport } from "../transport";
import {
  BranchIcon,
  ChevronIcon,
  FolderIcon,
  PlusIcon,
  RefreshIcon,
  STATE_LABEL,
  StateIcon,
} from "./icons";

/**
 * Arrow keys in the tree (#35): up/down move between rows, left/right collapse and expand a
 * project; Enter selects (the rows are buttons).
 */
function moveInTree(event: KeyboardEvent<HTMLElement>): void {
  const rows = [...event.currentTarget.querySelectorAll<HTMLElement>(".row-main")];
  const row = rows.indexOf(document.activeElement as HTMLElement);
  if (row < 0) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    rows[row + (event.key === "ArrowDown" ? 1 : -1)]?.focus();
  } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    const chevron = rows[row].parentElement?.querySelector<HTMLElement>("button.chevron");
    const open = chevron?.getAttribute("aria-expanded") === "true";
    if (open === (event.key === "ArrowLeft")) chevron?.click();
  } else {
    return;
  }
  event.preventDefault();
}

// The pending counter joins in 2.3. ponytail: plain list, add TanStack Virtual when trees get long.
export function Sidebar() {
  const projects = useHive((s) => s.projects);
  const list = Object.values(projects ?? {});
  return (
    <nav className="sidebar" aria-label="Projects" onKeyDown={moveInTree}>
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
  const agents = Object.values(useHive((s) => s.agents));
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
              <ul>
                {agents
                  .filter((a) => a.worktree === w.id)
                  .map((a) => (
                    <AgentRow key={a.id} agent={a} />
                  ))}
              </ul>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

/** The icon and the state's name under the row's title; the icon names it for screen readers. */
function StateLines({ state, title }: { state: AgentState; title: ReactNode }) {
  return (
    <>
      <StateIcon state={state} />
      <span className="agent-lines">
        <span className="label">{title}</span>
        <span className="state-label" data-state={state} aria-hidden="true">
          {STATE_LABEL[state]}
        </span>
      </span>
    </>
  );
}

/**
 * An agent, under the worktree the service placed it in, with its live subagents; clicking
 * either shows the agent's terminal. States are the service's (#37); until the first
 * `agent_state` arrives the agent shows as idle, as `SessionStart` leaves it.
 */
function AgentRow({ agent }: { agent: Agent }) {
  const tab = useHive((s) => s.tabs.find((t) => t.id === agent.terminal));
  const shown = useHive((s) => s.activeTab === agent.terminal);
  const status = useHive((s) => s.agentStates[agent.id]);
  const show = () => tab && activateTab(tab);
  return (
    <li>
      <div className="tree-row agent" title={agent.cwd ?? undefined} data-selected={shown}>
        <button type="button" className="row-main" aria-current={shown} onClick={show}>
          <StateLines state={status?.state ?? "idle"} title="Claude" />
        </button>
      </div>
      {status && status.subagents.length > 0 && (
        <ul>
          {status.subagents.map((sub) => (
            <li key={sub.id}>
              <div className="tree-row subagent">
                <button type="button" className="row-main" onClick={show}>
                  <StateLines
                    state={sub.state}
                    title={
                      <>
                        <span className="prefix">subagent: </span>
                        {sub.agent_type ?? "unknown"}
                      </>
                    }
                  />
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}
