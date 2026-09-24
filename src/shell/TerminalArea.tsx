import { useEffect, useRef } from "react";
import {
  activateTab,
  type HiveState,
  openModal,
  setRightPanel,
  type Tab,
  useHive,
  useTerminal,
} from "../store";
import { closeTerminal, mountTerminals, openTerminal, showTerminal } from "../terminals";
import { AddFolderIcon, CloseIcon, PanelIcon, PlusIcon, TerminalIcon } from "./icons";

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

/** The worktree at `path` and its project, as the service reports them. */
function find(s: HiveState, path: string) {
  for (const project of Object.values(s.projects ?? {})) {
    const worktree = project.worktrees.find((w) => w.id === path);
    if (worktree) return { worktree, project };
  }
  return null;
}

function TerminalTab({ tab }: { tab: Tab }) {
  const active = useHive((s) => s.activeTab === tab.id);
  const state = useTerminal(tab.id);
  const name = useHive((s) => find(s, tab.cwd)?.worktree.name ?? tab.cwd);
  // As in the prototype, the project tells apart tabs of worktrees with the same name.
  const project = useHive((s) => find(s, tab.cwd)?.project.name);
  const duplicate = useHive(
    (s) => s.tabs.filter((t) => (find(s, t.cwd)?.worktree.name ?? t.cwd) === name).length > 1,
  );
  return (
    <div className="tab" data-active={active} title={tab.cwd}>
      <button
        type="button"
        role="tab"
        aria-selected={active}
        className="tab-label"
        onClick={() => activateTab(tab)}
      >
        <TerminalIcon />
        <span className="tab-name">{name}</span>
        {duplicate && project && <span className="tab-hint">{project}</span>}
        {state?.unhooked && (
          <span
            className="tab-badge warn"
            title="Claude runs in this terminal without Hive's hooks: its state is not observed"
          >
            no hooks
          </span>
        )}
        {state?.exited && (
          <span className="tab-badge" title={`Exit code: ${state.code ?? "none (killed)"}`}>
            exited
          </span>
        )}
      </button>
      <button
        type="button"
        className="tab-close"
        title="Close terminal"
        aria-label={`Close terminal ${name}`}
        onClick={() => closeTerminal(tab.id)}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

/** Where xterm.js renders; the terminal manager owns everything inside it. */
function TerminalHost({ hidden }: { hidden: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const active = useHive((s) => s.activeTab);
  useEffect(() => mountTerminals(ref.current as HTMLDivElement), []);
  useEffect(() => showTerminal(active), [active]);
  return <div className="terminal-host" ref={ref} hidden={hidden} />;
}

// "+" opens a terminal in the selected worktree; Ctrl+Shift+T opens the worktree picker.
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
  const empty = useHive((s) => s.projects !== null && Object.keys(s.projects).length === 0);
  const tabs = useHive((s) => s.tabs);
  // A selected agent (F8) is not a folder: its worktree is, when the service placed it in one.
  const selected = useHive((s) => {
    const agent = s.agents[s.selection ?? ""];
    return agent ? agent.worktree : s.selection;
  });
  return (
    <section className="terminals" aria-label="Terminals">
      <div className="bar">
        <div className="tabs" role="tablist">
          {tabs.map((tab) => (
            <TerminalTab key={tab.id} tab={tab} />
          ))}
          <button
            type="button"
            className="ghost"
            title="New terminal (Ctrl+Shift+T)"
            disabled={selected === null}
            onClick={() => selected !== null && void openTerminal(selected)}
          >
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
      <div className="terminal-body">
        {empty && tabs.length === 0 && <EmptyState />}
        <TerminalHost hidden={tabs.length === 0} />
      </div>
    </section>
  );
}
