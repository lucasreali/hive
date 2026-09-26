import {
  FilePlusIcon,
  RobotIcon,
  SquareSplitHorizontalIcon,
  TerminalWindowIcon,
  XIcon,
} from "@phosphor-icons/react";
import {
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { useShallow } from "zustand/react/shallow";
import { closeChat, openChat } from "../chats";
import {
  activateTab,
  fileVisible,
  type HiveState,
  openFileDialog,
  openModal,
  selectedPlace,
  setRightPanel,
  showFile,
  shownSplit,
  shownTerminals,
  type Tab,
  useHive,
  useTerminal,
  visibleTabs,
} from "../store";
import {
  closeTerminal,
  mountTerminals,
  openTerminal,
  showTerminals,
  splitTerminal,
} from "../terminals";
import { isDirty } from "../viewer/buffer";
import { isMac, keyText } from "../window";
import { ChatView } from "./ChatView";
import {
  AddFolderIcon,
  ChatIcon,
  CloseIcon,
  FileIcon,
  ICON,
  PanelIcon,
  PlusIcon,
  StateIcon,
  TerminalIcon,
} from "./icons";
import { FileView, leaveFile } from "./RightPanel";
import { ResizeHandle } from "./resize";
import { TranscriptView } from "./TranscriptView";
import { ContextMenu } from "./WorktreeMenu";

/** Screen 1e: shown once the service said there are no projects. */
function EmptyState() {
  const mac = isMac();
  return (
    <div className="empty-state">
      <div className="empty-state-content">
        <AddFolderIcon />
        <div>
          <h2>No project open</h2>
          <p>Add a project to follow the agents running in its worktrees.</p>
        </div>
        <button type="button" className="primary" onClick={() => openModal("add-project")}>
          Add project <kbd>{keyText("Ctrl+Shift+O")}</kbd>
        </button>
        <div className="empty-state-example">
          <span>A project is a folder{mac ? "" : " inside WSL"}, for example:</span>
          <code>{mac ? "/Users/you" : "/home/user"}/projects/shop</code>
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

/**
 * A tab of the bar: its label shows it, its × closes it. Terminals and the open file share it.
 * With unsaved edits the × shows a dot instead, as in other editors, and turns back into the ×
 * on hover or keyboard focus, where it is about to be used.
 */
function TabItem(props: {
  active: boolean;
  /** Shown beside the active tab, in a split. */
  split?: boolean;
  onMenu?: (event: MouseEvent) => void;
  title: string;
  onShow: () => void;
  close: string;
  onClose: () => void;
  dirty?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className="tab"
      data-active={props.active}
      data-split={props.split || undefined}
      title={props.title}
    >
      <button
        type="button"
        role="tab"
        aria-selected={props.active}
        className="tab-label"
        onClick={props.onShow}
        onContextMenu={props.onMenu}
      >
        {props.children}
      </button>
      <button
        type="button"
        className="tab-close"
        data-dirty={props.dirty || undefined}
        title={props.dirty ? `${props.close} (unsaved changes)` : props.close}
        aria-label={props.dirty ? `${props.close} (unsaved changes)` : props.close}
        onClick={props.onClose}
      >
        {props.dirty && <span className="dirty" aria-hidden="true" />}
        <CloseIcon />
      </button>
    </div>
  );
}

/**
 * A terminal's (or a chat's) tab: the worktree's name, or, while a Claude agent runs in it, the
 * agent's state and its session's name (as in Orca). Tabs show only their worktree's, so no
 * project.
 */
function TerminalTab({ tab, onMenu }: { tab: Tab; onMenu: (menu: TabMenu) => void }) {
  const active = useHive((s) => s.activeTab === tab.id && !s.fileShown && !s.transcriptShown);
  const split = useHive((s) => {
    const shown = !s.fileShown && !s.transcriptShown && shownSplit(s);
    return !!shown && s.activeTab !== tab.id && (shown.left === tab.id || shown.right === tab.id);
  });
  const state = useTerminal(tab.id);
  const name = useHive((s) => find(s, tab.cwd)?.worktree.name ?? tab.cwd);
  const agent = useHive((s) => Object.values(s.agents).find((a) => a.terminal === tab.id)?.id);
  const agentState = useHive((s) => (agent ? (s.agentStates[agent]?.state ?? "idle") : null));
  const title = useHive((s) => (agent ? s.agentTitles[agent] : undefined));
  const chat = tab.kind === "chat";
  const ended = useHive((s) => !!s.chats[tab.id]?.closed);
  return (
    <TabItem
      active={active}
      split={split}
      // A chat has no split, so no menu.
      onMenu={
        chat
          ? undefined
          : (event) => {
              event.preventDefault();
              onMenu({ tab: tab.id, x: event.clientX, y: event.clientY });
            }
      }
      title={title ? `${title}\n${tab.cwd}` : tab.cwd}
      onShow={() => activateTab(tab)}
      close={`Close ${chat ? "chat" : "terminal"} ${title ?? name}`}
      onClose={() => (chat ? closeChat(tab.id) : closeTerminal(tab.id))}
    >
      {agentState ? <StateIcon state={agentState} /> : chat ? <ChatIcon /> : <TerminalIcon />}
      <span className="tab-name" data-agent={title ? true : undefined}>
        {title ?? name}
      </span>
      {state?.badge && (
        <span className="tab-badge label-badge" title="Set with hive badge">
          {state.badge}
        </span>
      )}
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
      {ended && <span className="tab-badge">ended</span>}
    </TabItem>
  );
}

/** The open file's tab, after the terminals; closing it asks first when edits are unsaved. */
function FileTab() {
  const open = useHive((s) => (fileVisible(s) ? s.openFile : null));
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
      dirty={dirty}
    >
      <FileIcon />
      <span className="tab-name">{name}</span>
    </TabItem>
  );
}

type TabMenu = { tab: number; x: number; y: number };

/** A terminal tab's context menu (right click): split it, or close it. */
function TerminalTabMenu({ menu, onClose }: { menu: TabMenu; onClose: () => void }) {
  const split = useHive((s) => {
    const shown = shownSplit(s);
    return !!shown && (shown.left === menu.tab || shown.right === menu.tab);
  });
  const act = (action: () => void) => () => {
    onClose();
    action();
  };
  return (
    <ContextMenu at={menu} label="Terminal" onClose={onClose}>
      <button type="button" role="menuitem" onClick={act(() => void splitTerminal(menu.tab))}>
        <SquareSplitHorizontalIcon {...ICON} />
        {split ? "Unsplit" : "Split right"}
      </button>
      <button type="button" role="menuitem" onClick={act(() => closeTerminal(menu.tab))}>
        <XIcon {...ICON} />
        Close terminal
      </button>
    </ContextMenu>
  );
}

/**
 * Where xterm.js renders; the terminal manager owns everything inside it. Split, the two panes
 * sit side by side with a divider between them (its position is a UI preference).
 */
function TerminalHost({ hidden }: { hidden: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const active = useHive((s) => s.activeTab);
  const panes = useHive(useShallow(shownTerminals));
  useEffect(() => mountTerminals(ref.current as HTMLDivElement), []);
  // Shown again after the file's tab: the focused terminal takes the focus back.
  useEffect(() => {
    if (!hidden) showTerminals(panes, active);
  }, [panes, active, hidden]);
  const split = panes.length > 1 && !hidden;
  return (
    <>
      <div className="terminal-host" ref={ref} hidden={hidden} data-split={split || undefined} />
      {split && <ResizeHandle side="split" />}
    </>
  );
}

/** A worktree without terminals yet: its tab bar is empty. */
function NoTerminals({ worktree }: { worktree: string }) {
  const name = useHive((s) => find(s, worktree)?.worktree.name ?? worktree);
  return (
    <div className="empty-state">
      <div className="empty-state-content centered">
        <TerminalWindowIcon size={32} weight="light" aria-hidden="true" />
        <p>No terminal in {name}</p>
        <button type="button" className="primary" onClick={() => void openTerminal(worktree)}>
          New terminal
        </button>
      </div>
    </div>
  );
}

/** The "+" menu's Agent: the in-app chat (7.3). */
const openAgent = (worktree: string) => void openChat(worktree);

/**
 * The tab bar's "+": a menu to open a terminal, an agent or a new file (its name asked first)
 * in `worktree`. Enter, Space or ↓ open it; Esc closes it and gives the focus back to "+".
 */
function NewTabButton({ worktree }: { worktree: string | null }) {
  const [plus, setPlus] = useState<HTMLButtonElement | null>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const close = useCallback(() => {
    setAt(null);
    plus?.focus();
  }, [plus]);
  const show = () => {
    const box = (plus as HTMLButtonElement).getBoundingClientRect();
    setAt({ x: box.left, y: box.bottom + 4 });
  };
  const act = (action: (worktree: string) => void) => () => {
    close();
    action(worktree as string);
  };
  return (
    <>
      <button
        ref={setPlus}
        type="button"
        className="ghost"
        title="New terminal, agent or file"
        aria-label="New terminal, agent or file"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        disabled={worktree === null}
        onClick={() => (at ? close() : show())}
        onKeyDown={(e) => {
          if (e.key !== "ArrowDown") return;
          e.preventDefault();
          show();
        }}
      >
        <PlusIcon size={14} />
      </button>
      {at && worktree !== null && (
        <ContextMenu at={at} label="New tab" onClose={close} anchor={plus}>
          <button type="button" role="menuitem" onClick={act((w) => void openTerminal(w))}>
            <TerminalWindowIcon {...ICON} />
            Terminal
          </button>
          <button type="button" role="menuitem" onClick={act(openAgent)}>
            <RobotIcon {...ICON} />
            Agent
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={act((w) => openFileDialog({ worktree: w, folder: "", path: null }))}
          >
            <FilePlusIcon {...ICON} />
            New file…
          </button>
        </ContextMenu>
      )}
    </>
  );
}

// "+" opens a terminal, an agent or a new file in the selected worktree; Ctrl+Shift+T opens the worktree picker. Only
// the selected worktree's tabs show (a project's are its main worktree's).
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
  const empty = useHive((s) => s.projects !== null && Object.keys(s.projects).length === 0);
  const tabs = useHive(useShallow(visibleTabs));
  const file = useHive((s) => (s.fileShown && fileVisible(s) ? s.openFile : null));
  const selected = useHive(selectedPlace);
  const transcript = useHive((s) => s.transcriptShown);
  const chat = useHive((s) =>
    !s.fileShown &&
    !s.transcriptShown &&
    visibleTabs(s).some((t) => t.id === s.activeTab && t.kind === "chat")
      ? s.activeTab
      : null,
  );
  const percent = useHive((s) => s.splitPercent);
  const [menu, setMenu] = useState<TabMenu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  return (
    <section className="terminals" aria-label="Terminals">
      <div className="bar">
        <div className="tabs" role="tablist" aria-label="Open terminals and files">
          {tabs.map((tab) => (
            <TerminalTab key={tab.id} tab={tab} onMenu={setMenu} />
          ))}
          <FileTab />
          <NewTabButton worktree={selected} />
        </div>
        <div className="tabs-actions">
          <button
            type="button"
            className="ghost"
            title={keyText("Files, diff and sessions (Ctrl+Shift+B)")}
            aria-pressed={open}
            onClick={() => setRightPanel(open ? null : "files")}
          >
            <PanelIcon />
          </button>
        </div>
      </div>
      {menu && <TerminalTabMenu menu={menu} onClose={closeMenu} />}
      <div className="terminal-body" style={{ "--split": percent / 100 } as CSSProperties}>
        {empty && tabs.length === 0 && !file && <EmptyState />}
        {!empty && selected !== null && tabs.length === 0 && !file && (
          <NoTerminals worktree={selected} />
        )}
        <TerminalHost hidden={tabs.length === 0 || !!file || !!transcript || chat !== null} />
        {chat !== null && <ChatView key={chat} id={chat} />}
        {file && <FileView worktree={file.worktree} />}
        {transcript && (
          <TranscriptView
            key={`${transcript.agent}\n${transcript.subagent}`}
            agent={transcript.agent}
            subagent={transcript.subagent}
          />
        )}
      </div>
    </section>
  );
}
