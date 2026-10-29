import {
  ArrowsLeftRightIcon,
  CaretRightIcon,
  ClipboardTextIcon,
  CopyIcon,
  EraserIcon,
  FilePlusIcon,
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
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useShallow } from "zustand/react/shallow";
import type { ProgressState } from "../protocol";
import { draggedFrom, useReorder } from "../reorder";
import {
  activateTab,
  dropOnPane,
  type FileTab as FileTabData,
  type HiveState,
  moveTab,
  muted,
  openFileDialog,
  openModal,
  pinFile,
  replacePane,
  selectedPlace,
  setOpenFile,
  setRightPanel,
  swapSides,
  type Tab,
  useHive,
  useTerminal,
} from "../store";
import {
  type BarItem,
  barItems,
  barKey,
  droppable,
  fileKey,
  fileTabState,
  fileVisible,
  shownSplit,
  shownTerminals,
  splitOf,
  tabWorktree,
  visibleTabs,
} from "../tabs";
import {
  closeTerminal,
  mountTerminals,
  openTerminal,
  showOpenFailure,
  showTerminals,
  splitTerminal,
  terminalAction,
  terminalAt,
} from "../terminals";
import { isDirty, isFor } from "../viewer/buffer";
import { isMac, keyText } from "../window";
import {
  AddFolderIcon,
  CloseIcon,
  FileIcon,
  ICON,
  PanelIcon,
  PlusIcon,
  StateIcon,
  TerminalIcon,
} from "./icons";
import { closeFile, FileView } from "./RightPanel";
import { ResizeHandle } from "./resize";
import { TerminalSearch } from "./TerminalSearch";
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

/** What a terminal's tab is called: its agent's session name, else its worktree's name. */
function tabLabel(s: HiveState, tab: Tab): string {
  const agent = Object.values(s.agents).find((a) => a.terminal === tab.id)?.id;
  return (agent && s.agentTitles[agent]) || (find(s, tab.cwd)?.worktree.name ?? tab.cwd);
}

/** The drag-and-drop props of a tab (`useReorder`), to move it along the bar (8.21). */
type Drag = ReturnType<ReturnType<typeof useReorder>>;

/**
 * A tab of the bar: its label shows it, its × closes it, dragging it moves it. Terminals and
 * files share it. With unsaved edits the × shows a dot instead, as in other editors, and
 * turns back into the × on hover or keyboard focus, where it is about to be used.
 */
function TabItem(props: {
  drag: Drag;
  active: boolean;
  /** Its side in a pair (20.5): the pair's two tabs are drawn as one group. */
  pair?: "left" | "right";
  /** A paired tab not focused: its pair shown ("view", the pane beside the active tab) or not. */
  split?: "view" | "paired";
  onMenu?: (event: MouseEvent) => void;
  title: string;
  onShow: () => void;
  /** A double click on the label (a file's: keeps its preview tab, 11.1). */
  onPin?: () => void;
  /** Read by screen readers after the label (e.g. "Preview"). */
  description?: string;
  close: string;
  onClose: () => void;
  dirty?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      {...props.drag}
      className="tab"
      data-active={props.active}
      data-pair={props.pair}
      data-split={props.split}
      title={props.title}
    >
      <button
        type="button"
        role="tab"
        aria-selected={props.active}
        className="tab-label"
        aria-description={props.description}
        onClick={props.onShow}
        onDoubleClick={props.onPin}
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
 * A terminal's tab: the worktree's name, or, while a Claude agent runs in it, the agent's state
 * and its session's name (as in Orca). Tabs show only their worktree's, so no project.
 */
function TerminalTab(props: { tab: Tab; onMenu: (menu: TabMenu) => void; drag: Drag }) {
  const { tab, onMenu } = props;
  const active = useHive((s) => s.activeTab === tab.id && !s.fileShown);
  // Its pair (20.5): its side, its partner's label, and whether the pair is shown.
  const pair = useHive(
    useShallow((s) => {
      const split = splitOf(s, tab.id);
      if (!split) return null;
      const partner = s.tabs.find(
        (t) => t.id === (split.left === tab.id ? split.right : split.left),
      );
      return {
        side: split.left === tab.id ? ("left" as const) : ("right" as const),
        partner: partner ? tabLabel(s, partner) : "",
        shown: !s.fileShown && shownSplit(s) === split,
      };
    }),
  );
  const state = useTerminal(tab.id);
  const name = useHive((s) => find(s, tab.cwd)?.worktree.name ?? tab.cwd);
  const agent = useHive((s) => Object.values(s.agents).find((a) => a.terminal === tab.id)?.id);
  const agentState = useHive((s) => (agent ? (s.agentStates[agent]?.state ?? "idle") : null));
  const seen = useHive((s) => muted(agent ? s.agentStates[agent] : null));
  const title = useHive((s) => (agent ? s.agentTitles[agent] : undefined));
  const label = title ?? name;
  const paired = pair ? `Split with ${pair.partner}` : undefined;
  const tip = title ? `${title}\n${tab.cwd}` : tab.cwd;
  return (
    <TabItem
      drag={props.drag}
      active={active}
      pair={pair?.side}
      split={pair && !active ? (pair.shown ? "view" : "paired") : undefined}
      description={paired}
      onMenu={(event) => {
        event.preventDefault();
        onMenu({ tab: tab.id, x: event.clientX, y: event.clientY });
      }}
      title={paired ? `${tip}\n${paired}` : tip}
      onShow={() => activateTab(tab)}
      close={`Close terminal ${label}`}
      onClose={() => closeTerminal(tab.id)}
    >
      {pair?.side === "left" && <SquareSplitHorizontalIcon className="tab-split" {...ICON} />}
      {agentState ? <StateIcon state={agentState} muted={seen} /> : <TerminalIcon />}
      <span className="tab-name" data-agent={title ? true : undefined}>
        {label}
      </span>
      {state?.badge && (
        <span className="tab-badge label-badge" title="Set with hive badge">
          {state.badge}
        </span>
      )}
      {state?.progress && <ProgressHint {...state.progress} />}
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

/** What a program's progress (OSC 9;4, 17.25) says, by state. */
const PROGRESS_TEXT: Record<Exclude<ProgressState, "none">, string> = {
  normal: "Progress",
  error: "Failed at",
  indeterminate: "Busy",
  paused: "Paused at",
};

/** The progress a program in the terminal reports (17.25): a small bar on its tab. */
function ProgressHint({ state, value }: { state: ProgressState; value: number }) {
  const busy = state === "indeterminate";
  const text = PROGRESS_TEXT[state as Exclude<ProgressState, "none">];
  const label = busy ? text : `${text} ${value}%`;
  return (
    <span
      className="tab-progress"
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={busy ? undefined : value}
      title={label}
      data-state={state}
      style={{ "--progress": `${value}%` } as CSSProperties}
    />
  );
}

/** An open file's tab (8.21); closing it asks first when its edits are unsaved. */
function FileTab({ file, drag }: { file: FileTabData; drag: Drag }) {
  const active = useHive((s) => s.fileShown && !!s.openFile && isFor(file, s.openFile));
  const dirty = useHive((s) => {
    const { edit } = fileTabState(s, file);
    return !!edit && isDirty(edit);
  });
  const name = file.path.slice(file.path.lastIndexOf("/") + 1);
  const preview = !!file.preview;
  return (
    <TabItem
      drag={drag}
      active={active}
      title={preview ? `${file.path}\nPreview: opening another file replaces it` : file.path}
      onShow={() => setOpenFile(file)}
      onPin={() => pinFile(file)}
      description={preview ? "Preview" : undefined}
      close={`Close file ${name}`}
      onClose={() => closeFile(file)}
      dirty={dirty}
    >
      <FileIcon />
      <span className="tab-name" data-preview={preview || undefined}>
        {name}
      </span>
    </TabItem>
  );
}

type TabMenu = { tab: number; x: number; y: number };

/** A menu item that closes the menu, then runs `action`. */
function MenuItem(props: {
  onClose: () => void;
  action: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={props.disabled}
      onClick={() => {
        props.onClose();
        props.action();
      }}
    >
      {props.children}
    </button>
  );
}

/** A menu item's key, after its label; nothing without one. */
const Keys = ({ keys }: { keys?: string }) =>
  keys ? (
    <>
      {" "}
      <kbd>{keys}</kbd>
    </>
  ) : null;

/**
 * "Split right" for terminal `tab`, or "Unsplit" when it is a shown pane (6.11); with its key
 * when `keys`.
 */
function SplitItem(props: { tab: number; onClose: () => void; keys?: boolean }) {
  const { tab, onClose } = props;
  const split = useHive((s) => !!splitOf(s, tab));
  return (
    <MenuItem onClose={onClose} action={() => showOpenFailure(splitTerminal(tab))}>
      <SquareSplitHorizontalIcon {...ICON} />
      {split ? "Unsplit" : "Split right"}
      <Keys keys={props.keys ? keyText("Ctrl+Shift+D") : undefined} />
    </MenuItem>
  );
}

/** A submenu beside its menu (on its left without room on its right), its first item focused. */
function Submenu({ label, children }: { label: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current as HTMLDivElement;
    if (el.getBoundingClientRect().right > window.innerWidth) el.dataset.flip = "";
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, []);
  return (
    <div ref={ref} className="context-menu submenu" role="menu" aria-label={label}>
      {children}
    </div>
  );
}

/**
 * For terminal `tab` in a pair (20.5): "Replace with ▸", a submenu of its worktree's other
 * terminals, one of which takes its pane; and "Swap sides".
 */
function PairItems({ tab, onClose }: { tab: number; onClose: () => void }) {
  const others = useHive(
    useShallow((s) => {
      const split = splitOf(s, tab);
      const own = s.tabs.find((t) => t.id === tab);
      if (!split || !own) return null;
      const place = tabWorktree(s, own);
      return s.tabs
        .filter((t) => t.id !== split.left && t.id !== split.right && tabWorktree(s, t) === place)
        .flatMap((t) => [t.id, tabLabel(s, t)]);
    }),
  );
  const [open, setOpen] = useState(false);
  if (!others) return null;
  return (
    <>
      <div className="submenu-anchor">
        <button
          type="button"
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={others.length === 0}
          onClick={() => setOpen(!open)}
          onKeyDown={(e) => {
            if (e.key !== "ArrowRight") return;
            e.preventDefault();
            setOpen(true);
          }}
        >
          <SquareSplitHorizontalIcon {...ICON} />
          Replace with
          <CaretRightIcon className="submenu-caret" {...ICON} />
        </button>
        {open && (
          <Submenu label="Replace with">
            {others.map((x, i) =>
              i % 2 ? null : (
                <MenuItem key={x} onClose={onClose} action={() => replacePane(tab, x as number)}>
                  <TerminalIcon />
                  {others[i + 1]}
                </MenuItem>
              ),
            )}
          </Submenu>
        )}
      </div>
      <MenuItem onClose={onClose} action={() => swapSides(tab)}>
        <ArrowsLeftRightIcon {...ICON} />
        Swap sides
      </MenuItem>
    </>
  );
}

/** A terminal tab's context menu (right click): split it, rearrange its pair, or close it. */
function TerminalTabMenu({ menu, onClose }: { menu: TabMenu; onClose: () => void }) {
  return (
    <ContextMenu at={menu} label="Terminal" onClose={onClose}>
      <SplitItem tab={menu.tab} onClose={onClose} />
      <PairItems tab={menu.tab} onClose={onClose} />
      <MenuItem onClose={onClose} action={() => closeTerminal(menu.tab)}>
        <XIcon {...ICON} />
        Close terminal
      </MenuItem>
    </ContextMenu>
  );
}

type PaneMenu = TabMenu & { selection: boolean };

/**
 * The menu of a terminal itself (17.20, right click or a two-finger tap in it): an app menu,
 * never the WebView's (#53). Copy, Paste and Clear do what their keys do.
 */
function TerminalPaneMenu({ menu, onClose }: { menu: PaneMenu; onClose: () => void }) {
  const mac = isMac();
  const run = (action: "copy" | "paste" | "clear") => () => terminalAction(menu.tab, action);
  return (
    <ContextMenu at={menu} label="Terminal" onClose={onClose}>
      <MenuItem onClose={onClose} action={run("copy")} disabled={!menu.selection}>
        <CopyIcon {...ICON} />
        Copy
        <Keys keys={mac ? "⌘C" : "Ctrl+Shift+C"} />
      </MenuItem>
      <MenuItem onClose={onClose} action={run("paste")}>
        <ClipboardTextIcon {...ICON} />
        Paste
        <Keys keys={mac ? "⌘V" : "Ctrl+Shift+V"} />
      </MenuItem>
      <MenuItem onClose={onClose} action={run("clear")}>
        <EraserIcon {...ICON} />
        Clear
        <Keys keys={mac ? "⌘K" : undefined} />
      </MenuItem>
      <hr />
      <SplitItem tab={menu.tab} onClose={onClose} keys />
      <PairItems tab={menu.tab} onClose={onClose} />
    </ContextMenu>
  );
}

/**
 * Where xterm.js renders; the terminal manager owns everything inside it. Split, the two panes
 * sit side by side with a divider between them (its position is a UI preference). A right
 * click in a terminal opens its menu. A tab of their worktree dragged from the bar onto a pane
 * takes it, or onto a terminal alone makes a pair on the side it is dropped (20.5), highlighted.
 */
function TerminalHost({ hidden }: { hidden: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const active = useHive((s) => s.activeTab);
  const panes = useHive(useShallow(shownTerminals));
  const share = useHive(leftShare);
  const [drop, setDrop] = useState<"left" | "right" | null>(null);
  const [menu, setMenu] = useState<PaneMenu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  useEffect(() => {
    const host = ref.current as HTMLDivElement;
    const onMenu = (event: globalThis.MouseEvent) => {
      const at = terminalAt(event.target);
      if (!at) return;
      event.preventDefault();
      setMenu({ tab: at.id, selection: at.selection, x: event.clientX, y: event.clientY });
    };
    const side = (event: DragEvent) => {
      const box = host.getBoundingClientRect();
      return event.clientX < box.left + box.width * leftShare(useHive.getState())
        ? "left"
        : "right";
    };
    const dragged = () => {
      const key = draggedFrom("tabs");
      return key !== null && droppable(useHive.getState(), key) ? key : null;
    };
    const onOver = (event: DragEvent) => {
      if (!dragged()) return;
      event.preventDefault();
      setDrop(side(event));
    };
    const onLeave = (event: DragEvent) => {
      if (!host.contains(event.relatedTarget as Node | null)) setDrop(null);
    };
    const onDrop = (event: DragEvent) => {
      setDrop(null);
      const key = dragged();
      if (!key) return;
      event.preventDefault();
      dropOnPane(key, side(event));
    };
    const listeners = { contextmenu: onMenu, dragover: onOver, dragleave: onLeave, drop: onDrop };
    for (const [type, on] of Object.entries(listeners))
      host.addEventListener(type, on as EventListener);
    const unmount = mountTerminals(host);
    return () => {
      for (const [type, on] of Object.entries(listeners))
        host.removeEventListener(type, on as EventListener);
      unmount();
    };
  }, []);
  // Shown again after the file's tab: the focused terminal takes the focus back.
  useEffect(() => {
    if (!hidden) showTerminals(panes, active);
  }, [panes, active, hidden]);
  const split = panes.length > 1 && !hidden;
  return (
    <>
      <div className="terminal-host" ref={ref} hidden={hidden} data-split={split || undefined} />
      {drop && (
        <div
          className="pane-drop"
          data-side={drop}
          style={{ "--drop": share } as CSSProperties}
          aria-hidden="true"
        />
      )}
      {split && <ResizeHandle side="split" />}
      <TerminalSearch />
      {menu && <TerminalPaneMenu menu={menu} onClose={closeMenu} />}
    </>
  );
}

/** The left pane's share of the terminal area: the divider's in a split, else half (a drop there makes a pair). */
const leftShare = (s: HiveState) => (shownTerminals(s).length > 1 ? s.splitPercent / 100 : 0.5);

/** A worktree without terminals yet: its tab bar is empty. */
function NoTerminals({ worktree }: { worktree: string }) {
  const name = useHive((s) => find(s, worktree)?.worktree.name ?? worktree);
  return (
    <div className="empty-state">
      <div className="empty-state-content centered">
        <TerminalWindowIcon size={32} weight="light" aria-hidden="true" />
        <p>No terminal in {name}</p>
        <button
          type="button"
          className="primary"
          onClick={() => showOpenFailure(openTerminal(worktree))}
        >
          New terminal
        </button>
      </div>
    </div>
  );
}

/**
 * The tab bar's "+": a menu to open a terminal or a new file (its name asked first)
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
        title="New terminal or file"
        aria-label="New terminal or file"
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
          <button
            type="button"
            role="menuitem"
            onClick={act((w) => showOpenFailure(openTerminal(w)))}
          >
            <TerminalWindowIcon {...ICON} />
            Terminal
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

// "+" opens a terminal or a new file in the selected worktree; Ctrl+Shift+T opens the worktree picker. Only
// the selected worktree's tabs show (a project's are its main worktree's).
export function TerminalArea() {
  const open = useHive((s) => s.rightPanel === "files");
  const empty = useHive((s) => s.projects !== null && Object.keys(s.projects).length === 0);
  const tabs = useHive(useShallow(visibleTabs));
  // One selector, so the bar is built once per store update: each item, then its key.
  const bar = useHive(useShallow((s) => barItems(s).flatMap((item) => [item, barKey(s, item)])));
  const items = bar.filter((x): x is BarItem => typeof x !== "string");
  const keys = bar.filter((x): x is string => typeof x === "string");
  const drag = useReorder("tabs", moveTab, true);
  const file = useHive((s) => (s.fileShown && fileVisible(s) ? s.openFile : null));
  const selected = useHive(selectedPlace);
  const percent = useHive((s) => s.splitPercent);
  const [menu, setMenu] = useState<TabMenu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  return (
    <section className="terminals" aria-label="Terminals">
      <div className="bar">
        <div
          className="tabs"
          role="tablist"
          aria-label="Open terminals and files"
          {...drag.end(keys.at(-1))}
        >
          {items.map((item, i) => {
            const key = keys[i] as string;
            return "path" in item ? (
              <FileTab key={fileKey(item)} file={item} drag={drag(key)} />
            ) : (
              <TerminalTab key={item.id} tab={item} onMenu={setMenu} drag={drag(key)} />
            );
          })}
          <NewTabButton worktree={selected} />
        </div>
        <div className="tabs-actions">
          <button
            type="button"
            className="ghost unpressed-look"
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
        <TerminalHost hidden={tabs.length === 0 || !!file} />
        {file && <FileView worktree={file.worktree} />}
      </div>
    </section>
  );
}
