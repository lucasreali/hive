import { PlusIcon as NewChatIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, lazy, type ReactNode, Suspense, useEffect, useRef } from "react";
import {
  type Agent,
  type AgentState,
  activateTab,
  mostUrgent,
  openMenu,
  openModal,
  type Project,
  type Subagent,
  select,
  toggleCollapsed,
  useHive,
  type Worktree,
} from "../store";
import { openClaude } from "../terminals";
import { transport } from "../transport";
import { keyText } from "../window";
import {
  BranchIcon,
  ChevronIcon,
  FolderIcon,
  PlusIcon,
  RefreshIcon,
  STATE_LABEL,
  StateIcon,
} from "./icons";
import { ResizeHandle } from "./resize";

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

/** A collapsed node's most urgent state inside (rule 1), by the service's urgency. */
function Rollup({ agents }: { agents: (a: Agent) => boolean }) {
  const state = useHive((s) => mostUrgent(s, Object.values(s.agents).filter(agents)));
  return state && <StateIcon state={state} />;
}

// ponytail: plain list, add TanStack Virtual when trees get long.
export function Sidebar() {
  const projects = useHive((s) => s.projects);
  const width = useHive((s) => s.sidebarWidth);
  const list = Object.values(projects ?? {});
  return (
    <nav className="sidebar" aria-label="Projects" onKeyDown={moveInTree} style={{ width }}>
      <ResizeHandle side="sidebar" />
      <div className="bar">
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
            title={keyText("Add project (Ctrl+Shift+O)")}
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
  const states = Object.values(useHive((s) => s.agentStates));
  // A subagent's own worktree shows as its parent row instead (#22), unless an agent runs there.
  const owned = new Set(states.flatMap((st) => st.subagents.map((sub) => sub.worktree)));
  const shown = project.worktrees.filter(
    (w) => !owned.has(w.id) || agents.some((a) => a.worktree === w.id),
  );
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
          {!open && <Rollup agents={(a) => project.worktrees.some((w) => w.id === a.worktree)} />}
        </button>
        <button
          type="button"
          className="new-worktree"
          title={keyText("New worktree (Ctrl+Shift+N)")}
          onClick={() => openModal("new-worktree", project.id)}
        >
          <PlusIcon size={10} />
          New worktree
        </button>
      </div>
      {open && (
        <ul>
          {project.error && <li className="tree-error">{project.error}</li>}
          {shown.map((w) => (
            <WorktreeNode
              key={w.id}
              worktree={w}
              agents={agents.filter((a) => a.worktree === w.id)}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

/** A worktree and its agents; it collapses only when it has agents, as in the prototype. */
function WorktreeNode({ worktree: w, agents }: { worktree: Worktree; agents: Agent[] }) {
  const open = useHive((s) => !s.collapsed[`worktree:${w.id}`]);
  const selected = useHive((s) => s.selection === w.id);
  return (
    <li>
      <div className="tree-row worktree" title={w.path} data-selected={selected}>
        {agents.length > 0 ? (
          <button
            type="button"
            className="chevron"
            aria-label={`${open ? "Collapse" : "Expand"} ${w.name}`}
            aria-expanded={open}
            onClick={() => toggleCollapsed(`worktree:${w.id}`)}
          >
            <ChevronIcon open={open} />
          </button>
        ) : (
          <span className="chevron" />
        )}
        <button
          type="button"
          className="row-main"
          aria-current={selected}
          aria-haspopup="menu"
          onClick={() => select(w.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            // The keyboard's menu key gives no pointer position: open under the row.
            const row = e.currentTarget.getBoundingClientRect();
            const keyboard = e.clientX === 0 && e.clientY === 0;
            const x = keyboard ? row.left + 24 : e.clientX;
            const y = keyboard ? row.bottom : e.clientY;
            openMenu({ worktree: w.id, x, y });
          }}
        >
          <BranchIcon />
          <span className="label">{w.name}</span>
          {!open && <Rollup agents={(a) => a.worktree === w.id} />}
        </button>
        <button
          type="button"
          className="new-chat"
          title="New chat: a terminal running claude"
          aria-label={`New chat in ${w.name}`}
          onClick={() => void openClaude(w.path)}
        >
          <NewChatIcon size={12} weight="bold" aria-hidden="true" />
        </button>
      </div>
      {open && (
        <ul>
          {agents.map((a) => (
            <AgentRow key={a.id} agent={a} />
          ))}
        </ul>
      )}
    </li>
  );
}

/** Claude's mark from the icon library, which cannot be tree-shaken: it loads in its own chunk, shared with the file tree. */
const ClaudeIcon = lazy(async () => {
  const { Claude } = await import("@react-symbols/icons/files");
  return {
    default: () => <Claude className="agent-icon" width={13} height={13} aria-hidden="true" />,
  };
});

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
  const picked = useHive((s) => s.selection === agent.id);
  const shown = useHive((s) => s.activeTab === agent.terminal) || picked;
  const status = useHive((s) => s.agentStates[agent.id]);
  const show = () => tab && activateTab(tab);
  const row = useRef<HTMLDivElement>(null);
  // F8 picks an agent that may be out of view.
  useEffect(() => {
    if (picked) row.current?.scrollIntoView({ block: "nearest" });
  }, [picked]);
  return (
    <li>
      <div
        ref={row}
        className="tree-row agent"
        title={agent.cwd ?? undefined}
        data-selected={shown}
      >
        <button type="button" className="row-main" aria-current={shown} onClick={show}>
          <StateLines
            state={status?.state ?? "idle"}
            title={
              <>
                <Suspense fallback={<span className="agent-icon" />}>
                  <ClaudeIcon />
                </Suspense>
                Claude
              </>
            }
          />
        </button>
      </div>
      {status && status.subagents.length > 0 && (
        <ul>
          {status.subagents.map((sub) => (
            <SubagentNode key={sub.id} sub={sub} show={show} />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * A subagent; with a worktree of its own, that worktree is its parent row (Project → Worktree →
 * Agent at every level, #22). Clicking either row shows the agent's terminal.
 */
function SubagentNode({ sub, show }: { sub: Subagent; show: () => void }) {
  const w = useHive((s) =>
    Object.values(s.projects ?? {})
      .flatMap((p) => p.worktrees)
      .find((w) => w.id === sub.worktree),
  );
  const row = (
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
  );
  // Until the service's next `projects` lists its worktree, the subagent stays in place.
  if (!w) return <li>{row}</li>;
  return (
    <li>
      <div className="tree-row own-worktree" title={w.path}>
        <button type="button" className="row-own" tabIndex={-1} onClick={show}>
          <BranchIcon />
          <span className="label">{w.name}</span>
        </button>
      </div>
      <ul className="owned">
        <li>{row}</li>
      </ul>
    </li>
  );
}
