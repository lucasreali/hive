import { PlusIcon as NewChatIcon } from "@phosphor-icons/react";
import {
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  useEffect,
  useRef,
  useSyncExternalStore,
} from "react";
import {
  type Agent,
  type AgentState,
  type AgentUsage,
  activateTab,
  type Doing,
  mostUrgent,
  openMenu,
  openModal,
  openProjectMenu,
  type Project,
  type Subagent,
  select,
  showTranscript,
  toggleCollapsed,
  useHive,
  type Worktree,
  type WorktreeStatus,
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
          aria-haspopup="menu"
          onClick={() => select(project.id)}
          onContextMenu={(e) => openProjectMenu({ project: project.id, ...menuAt(e) })}
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

/** Where a row's context menu opens: at the pointer, or under the row for the menu key. */
function menuAt(e: MouseEvent<HTMLElement>): { x: number; y: number } {
  e.preventDefault();
  // The keyboard's menu key gives no pointer position.
  const row = e.currentTarget.getBoundingClientRect();
  const keyboard = e.clientX === 0 && e.clientY === 0;
  return { x: keyboard ? row.left + 24 : e.clientX, y: keyboard ? row.bottom : e.clientY };
}

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

/** The service's status of a worktree as small badges, each explained by its tooltip. */
function Health({ status: s }: { status: WorktreeStatus }) {
  return (
    <span className="health">
      {!!s.ahead && (
        <span title={`${plural(s.ahead, "commit")} not on the main worktree's branch`}>
          ↑{s.ahead}
        </span>
      )}
      {!!s.behind && (
        <span title={`${plural(s.behind, "commit")} on the main worktree's branch not here`}>
          ↓{s.behind}
        </span>
      )}
      {s.changes > 0 && <span title={`${plural(s.changes, "changed file")}`}>●{s.changes}</span>}
      {s.merged && <span title="Every commit is on the main worktree's branch">merged</span>}
    </span>
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
          onContextMenu={(e) => openMenu({ worktree: w.id, ...menuAt(e) })}
        >
          <BranchIcon />
          <span className="label">{w.name}</span>
          {!open && <Rollup agents={(a) => a.worktree === w.id} />}
        </button>
        {w.status && <Health status={w.status} />}
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

/** How long a state has lasted, from its start and now (ms): "12s", "3m", "1h". */
export function elapsed(since: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - since) / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  return `${Math.floor(seconds / 3600)}h`;
}

/** One clock for every row: a single 1 s timer, running while any row shows a time. */
const clock = { now: Date.now(), rows: new Set<() => void>(), timer: 0 as unknown };
function onTick(row: () => void) {
  if (clock.rows.size === 0) {
    clock.now = Date.now();
    clock.timer = setInterval(() => {
      clock.now = Date.now();
      for (const r of clock.rows) r();
    }, 1000);
  }
  clock.rows.add(row);
  return () => {
    clock.rows.delete(row);
    if (clock.rows.size === 0) clearInterval(clock.timer as number);
  };
}
export const useNow = () => useSyncExternalStore(onTick, () => clock.now);

/** The time in the state and what it is doing (muted, after the state's name). */
function Meta({ doing }: { doing: Doing }) {
  const now = useNow();
  const time = elapsed(doing.since_ms, now);
  return (
    <span className="state-meta">{doing.activity ? `${time} · ${doing.activity}` : time}</span>
  );
}

/**
 * The icon and the state's name under the row's title; the icon names it for screen readers.
 * Once the service sent it, the time in the state and the activity follow the name.
 */
function StateLines({
  state,
  doing,
  usage,
  title,
}: {
  state: AgentState;
  doing?: Doing;
  usage?: AgentUsage;
  title: ReactNode;
}) {
  return (
    <>
      <StateIcon state={state} />
      <span className="agent-lines">
        <span className="label">{title}</span>
        <span className="state-line">
          <span className="state-label" data-state={state} aria-hidden="true">
            {STATE_LABEL[state]}
          </span>
          {doing && <Meta doing={doing} />}
          {usage && (
            <span className="state-ctx" title={`${usage.context_tokens} context tokens`}>
              ctx {Math.round((usage.context_tokens / usage.context_limit) * 100)}%
            </span>
          )}
        </span>
      </span>
    </>
  );
}

/**
 * An agent, under the worktree the service placed it in, with its live subagents; clicking the
 * agent shows its terminal, clicking a subagent its conversation (6.10). States are the service's (#37); until the first
 * `agent_state` arrives the agent shows as idle, as `SessionStart` leaves it.
 */
function AgentRow({ agent }: { agent: Agent }) {
  const tab = useHive((s) => s.tabs.find((t) => t.id === agent.terminal));
  const picked = useHive((s) => s.selection === agent.id);
  // While a subagent's conversation shows, its row is the selected one.
  const covered = useHive((s) => s.transcriptShown !== null);
  const shown = (useHive((s) => s.activeTab === agent.terminal) || picked) && !covered;
  const status = useHive((s) => s.agentStates[agent.id]);
  const usage = useHive((s) => s.agentUsage[agent.id]);
  // The session's name, as on its tab; until Claude names it, just "Claude".
  const name = useHive((s) => s.agentTitles[agent.id]) ?? "Claude";
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
          <StateLines state={status?.state ?? "idle"} doing={status} usage={usage} title={name} />
        </button>
      </div>
      {status && status.subagents.length > 0 && (
        <ul>
          {status.subagents.map((sub) => (
            <SubagentNode key={sub.id} agent={agent.id} sub={sub} />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * A subagent; with a worktree of its own, that worktree is its parent row (Project → Worktree →
 * Agent at every level, #22). Clicking either row shows the subagent's conversation.
 */
function SubagentNode({ agent, sub }: { agent: string; sub: Subagent }) {
  const w = useHive((s) =>
    Object.values(s.projects ?? {})
      .flatMap((p) => p.worktrees)
      .find((w) => w.id === sub.worktree),
  );
  const shown = useHive(
    (s) => s.transcriptShown?.agent === agent && s.transcriptShown.subagent === sub.id,
  );
  const show = () => showTranscript(agent, sub.id);
  const row = (
    <div className="tree-row subagent" data-selected={shown}>
      <button type="button" className="row-main" aria-current={shown} onClick={show}>
        <StateLines
          state={sub.state}
          doing={sub}
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
