import { type ReactNode, useEffect, useRef } from "react";
import {
  activateTab,
  type HiveState,
  openModal,
  selectedPlace,
  setRightPanel,
  showFile,
  type Tab,
  useHive,
  useTerminal,
} from "../store";
import { closeTerminal, mountTerminals, openTerminal, showTerminal } from "../terminals";
import { isDirty } from "../viewer/buffer";
import { AddFolderIcon, CloseIcon, FileIcon, PanelIcon, PlusIcon, TerminalIcon } from "./icons";
import { FileView, leaveFile } from "./RightPanel";

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

/** A tab of the bar: its label shows it, its × closes it. Terminals and the open file share it. */
function TabItem(props: {
  active: boolean;
  title: string;
  onShow: () => void;
  close: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <div className="tab" data-active={props.active} title={props.title}>
      <button
        type="button"
        role="tab"
        aria-selected={props.active}
        className="tab-label"
        onClick={props.onShow}
      >
        {props.children}
      </button>
      <button
        type="button"
        className="tab-close"
        title={props.close}
        aria-label={props.close}
        onClick={props.onClose}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

function TerminalTab({ tab }: { tab: Tab }) {
  const active = useHive((s) => s.activeTab === tab.id && !s.fileShown);
  const state = useTerminal(tab.id);
  const name = useHive((s) => find(s, tab.cwd)?.worktree.name ?? tab.cwd);
  // As in the prototype, the project tells apart tabs of worktrees with the same name.
  const project = useHive((s) => find(s, tab.cwd)?.project.name);
  const duplicate = useHive(
    (s) => s.tabs.filter((t) => (find(s, t.cwd)?.worktree.name ?? t.cwd) === name).length > 1,
  );
  return (
    <TabItem
      active={active}
      title={tab.cwd}
      onShow={() => activateTab(tab)}
      close={`Close terminal ${name}`}
      onClose={() => closeTerminal(tab.id)}
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
    </TabItem>
  );
}

/** The open file's tab, after the terminals; closing it asks first when edits are unsaved. */
function FileTab() {
  const open = useHive((s) => s.openFile);
  const active = useHive((s) => s.fileShown);
  const dirty = useHive((s) => !!s.edit && isDirty(s.edit));
  if (!open) return null;
  const name = open.path.slice(open.path.lastIndexOf("/") + 1);
  return (
    <TabItem
      active={active}
      title={open.path}
      onShow={showFile}
      close={`Close file ${name}`}
      onClose={() => leaveFile(null)}
    >
      <FileIcon />
      <span className="tab-name">{name}</span>
      {dirty && (
        <span className="dirty" role="img" aria-label="Unsaved changes" title="Unsaved changes" />
      )}
    </TabItem>
  );
}

/** Where xterm.js renders; the terminal manager owns everything inside it. */
function TerminalHost({ hidden }: { hidden: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const active = useHive((s) => s.activeTab);
  useEffect(() => mountTerminals(ref.current as HTMLDivElement), []);
  // Shown again after the file's tab: the terminal takes the focus back.
  useEffect(() => {
    if (!hidden) showTerminal(active);
  }, [active, hidden]);
  return <div className="terminal-host" ref={ref} hidden={hidden} />;
}

// "+" opens a terminal in the selected worktree; Ctrl+Shift+T opens the worktree picker.
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
  const empty = useHive((s) => s.projects !== null && Object.keys(s.projects).length === 0);
  const tabs = useHive((s) => s.tabs);
  const file = useHive((s) => (s.fileShown ? s.openFile : null));
  const selected = useHive(selectedPlace);
  return (
    <section className="terminals" aria-label="Terminals">
      <div className="bar">
        <div className="tabs" role="tablist">
          {tabs.map((tab) => (
            <TerminalTab key={tab.id} tab={tab} />
          ))}
          <FileTab />
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
        {empty && tabs.length === 0 && !file && <EmptyState />}
        <TerminalHost hidden={tabs.length === 0 || !!file} />
        {file && <FileView worktree={file.worktree} />}
      </div>
    </section>
  );
}
