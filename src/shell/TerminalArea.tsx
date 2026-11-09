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
  accountLine,
  activateTab,
  dropOnPane,
  type FileTab as FileTabData,
  type HiveState,
  moveTab,
  muted,
  openFileDialog,
  openModal,
  type PaneId,
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
  inBarOrder,
  itemWorktree,
  paneFile,
  paneIdOf,
  shownPanes,
  shownSplit,
  shownTerminals,
  splitOf,
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

/** A file's name, as its tab shows it. */
const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** What a tab of the bar is called. */
const itemLabel = (s: HiveState, item: BarItem) =>
  "path" in item ? fileName(item.path) : tabLabel(s, item);

/**
 * Pane `id`'s pair as its tab shows it (20.5): its side, its partner's label, and whether the pair
 * is shown; null outside every pair.
 */
const usePair = (id: PaneId) =>
  useHive(
    useShallow((s) => {
      const split = splitOf(s, id);
      if (!split) return null;
      const other = split.left === id ? split.right : split.left;
      const partner = [...s.tabs, ...s.openFiles].find((t) => paneIdOf(t) === other);
      return {
        side: split.left === id ? ("left" as const) : ("right" as const),
        partner: partner ? itemLabel(s, partner) : "",
        shown: shownSplit(s) === split,
      };
    }),
  );

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
  const pair = usePair(tab.id);
  const state = useTerminal(tab.id);
  const name = useHive((s) => find(s, tab.cwd)?.worktree.name ?? tab.cwd);
  const agent = useHive((s) => Object.values(s.agents).find((a) => a.terminal === tab.id)?.id);
  const agentState = useHive((s) => (agent ? (s.agentStates[agent]?.state ?? "idle") : null));
  const seen = useHive((s) => muted(agent ? s.agentStates[agent] : null));
  const title = useHive((s) => (agent ? s.agentTitles[agent] : undefined));
  const label = title ?? name;
  const paired = pair ? `Split with ${pair.partner}` : undefined;
  const account = useHive((s) => accountLine(s, tab.id));
  const tip = [title, tab.cwd, account].filter(Boolean).join("\n");
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
function FileTab(props: { file: FileTabData; onMenu: (menu: TabMenu) => void; drag: Drag }) {
  const { file, onMenu, drag } = props;
  const pair = usePair(fileKey(file));
  const active = useHive((s) => s.fileShown && !!s.openFile && isFor(file, s.openFile));
  const dirty = useHive((s) => {
    const { edit } = fileTabState(s, file);
    return !!edit && isDirty(edit);
  });
  const name = fileName(file.path);
  const preview = !!file.preview;
  const paired = pair ? `Split with ${pair.partner}` : undefined;
  const tip = preview ? `${file.path}\nPreview: opening another file replaces it` : file.path;
  return (
    <TabItem
      drag={drag}
      active={active}
      pair={pair?.side}
      split={pair && !active ? (pair.shown ? "view" : "paired") : undefined}
      onMenu={(event) => {
        event.preventDefault();
        onMenu({ tab: fileKey(file), x: event.clientX, y: event.clientY });
      }}
      title={paired ? `${tip}\n${paired}` : tip}
      onShow={() => setOpenFile(file)}
      onPin={() => pinFile(file)}
      description={[preview ? "Preview" : "", paired ?? ""].filter(Boolean).join(". ") || undefined}
      close={`Close file ${name}`}
      onClose={() => closeFile(file)}
      dirty={dirty}
    >
      {pair?.side === "left" && <SquareSplitHorizontalIcon className="tab-split" {...ICON} />}
      <FileIcon />
      <span className="tab-name" data-preview={preview || undefined}>
        {name}
      </span>
    </TabItem>
  );
}

type TabMenu = { tab: PaneId; x: number; y: number };

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
 * "Split right" for tab `tab`, or "Unsplit" when it is a shown pane (6.11); with its key when
 * `keys`.
 */
function SplitItem(props: { tab: PaneId; onClose: () => void; keys?: boolean }) {
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
export function Submenu({ label, children }: { label: string; children: ReactNode }) {
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
 * What can take a pane of `tab`'s pair (20.5, 25.1): the other tabs of its worktree, terminals and
 * files, in the bar's order, as [pane, label] after one another. Tabs of the same name (a
 * worktree's plain terminals) also say their place in the bar. Null outside every pair.
 */
function replacements(s: HiveState, tab: PaneId): (PaneId | string)[] | null {
  const split = splitOf(s, tab);
  const own = [...s.tabs, ...s.openFiles].find((t) => paneIdOf(t) === tab);
  if (!split || !own) return null;
  const place = itemWorktree(s, own);
  const same = inBarOrder(
    s,
    [...s.tabs, ...s.openFiles].filter((t) => itemWorktree(s, t) === place),
  );
  const labels = same.map((t) => itemLabel(s, t));
  return same.flatMap((t, i) => {
    const pane = paneIdOf(t);
    if (pane === split.left || pane === split.right) return [];
    const label = labels[i] as string;
    const repeated = labels.indexOf(label) !== labels.lastIndexOf(label);
    return [pane, repeated ? `${label} (tab ${i + 1})` : label];
  });
}

/**
 * For tab `tab` in a pair (20.5): "Replace with ▸", a submenu of its worktree's other tabs,
 * terminals and files (25.1), one of which takes its pane; and "Swap sides".
 */
function PairItems({ tab, onClose }: { tab: PaneId; onClose: () => void }) {
  const others = useHive(useShallow((s) => replacements(s, tab)));
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
          title={others.length === 0 ? "No other tab in this worktree" : undefined}
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
                <MenuItem key={x} onClose={onClose} action={() => replacePane(tab, x)}>
                  {typeof x === "number" ? <TerminalIcon /> : <FileIcon />}
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

/** With more than one Claude account, the one terminal `tab` runs as (21.7), atop its menus. */
function AccountNote({ tab }: { tab: number }) {
  const account = useHive((s) => accountLine(s, tab));
  if (!account) return null;
  return (
    <>
      <div className="menu-note">{account}</div>
      <hr />
    </>
  );
}

/** A terminal tab's context menu (right click): split it, rearrange its pair, or close it. */
function TerminalTabMenu(props: { tab: number; at: TabMenu; onClose: () => void }) {
  const { tab, at, onClose } = props;
  return (
    <ContextMenu at={at} label="Terminal" onClose={onClose}>
      <AccountNote tab={tab} />
      <SplitItem tab={tab} onClose={onClose} />
      <PairItems tab={tab} onClose={onClose} />
      <MenuItem onClose={onClose} action={() => closeTerminal(tab)}>
        <XIcon {...ICON} />
        Close terminal
      </MenuItem>
    </ContextMenu>
  );
}

/** A file tab's context menu (25.1, also on its pane's header), as a terminal's. */
function FileTabMenu({ at, onClose }: { at: TabMenu; onClose: () => void }) {
  const file = useHive((s) => paneFile(s, at.tab));
  if (!file) return null;
  return (
    <ContextMenu at={at} label="File" onClose={onClose}>
      <SplitItem tab={at.tab} onClose={onClose} />
      <PairItems tab={at.tab} onClose={onClose} />
      <MenuItem onClose={onClose} action={() => closeFile(file)}>
        <XIcon {...ICON} />
        Close file
      </MenuItem>
    </ContextMenu>
  );
}

type PaneMenu = { tab: number; x: number; y: number; selection: boolean };

/**
 * The menu of a terminal itself (17.20, right click or a two-finger tap in it): an app menu,
 * never the WebView's (#53). Copy, Paste and Clear do what their keys do.
 */
function TerminalPaneMenu({ menu, onClose }: { menu: PaneMenu; onClose: () => void }) {
  const mac = isMac();
  const run = (action: "copy" | "paste" | "clear") => () => terminalAction(menu.tab, action);
  return (
    <ContextMenu at={menu} label="Terminal" onClose={onClose}>
      <AccountNote tab={menu.tab} />
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
 * sit side by side; beside a file (25.1), it takes its side of the divider (`half`). A right
 * click in a terminal opens its menu. A tab of the worktree dragged from the bar onto a pane, file
 * or terminal, takes it, or onto a tab alone makes a pair on the side it is dropped (20.5, 25.1),
 * highlighted: the whole terminal body takes the drop, ahead of an editor under it.
 */
function TerminalHost(props: { hidden: boolean; half?: "left" | "right" }) {
  const { hidden, half } = props;
  const ref = useRef<HTMLDivElement>(null);
  // The terminal to give the keyboard: none while a file's pane has the focus.
  const active = useHive((s) => (s.fileShown ? null : s.activeTab));
  const panes = useHive(useShallow(shownTerminals));
  const share = useHive(leftShare);
  const [drop, setDrop] = useState<"left" | "right" | null>(null);
  const [menu, setMenu] = useState<PaneMenu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  useEffect(() => {
    const host = ref.current as HTMLDivElement;
    const body = host.parentElement as HTMLElement;
    const onMenu = (event: globalThis.MouseEvent) => {
      const at = terminalAt(event.target);
      if (!at) return;
      event.preventDefault();
      setMenu({ tab: at.id, selection: at.selection, x: event.clientX, y: event.clientY });
    };
    const side = (event: DragEvent) => {
      const box = body.getBoundingClientRect();
      return event.clientX < box.left + box.width * leftShare(useHive.getState())
        ? "left"
        : "right";
    };
    const dragged = () => {
      const key = draggedFrom("tabs");
      return key !== null && droppable(useHive.getState(), key) !== null ? key : null;
    };
    const onOver = (event: DragEvent) => {
      if (!dragged()) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      setDrop(side(event));
    };
    const onLeave = (event: DragEvent) => {
      if (!body.contains(event.relatedTarget as Node | null)) setDrop(null);
    };
    const onDrop = (event: DragEvent) => {
      setDrop(null);
      const key = dragged();
      if (!key) return;
      event.preventDefault();
      event.stopPropagation();
      dropOnPane(key, side(event));
    };
    const drops = { dragover: onOver, dragleave: onLeave, drop: onDrop };
    host.addEventListener("contextmenu", onMenu);
    for (const [type, on] of Object.entries(drops))
      body.addEventListener(type, on as EventListener, true);
    const unmount = mountTerminals(host);
    return () => {
      host.removeEventListener("contextmenu", onMenu);
      for (const [type, on] of Object.entries(drops))
        body.removeEventListener(type, on as EventListener, true);
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
      <div
        className="terminal-host"
        ref={ref}
        hidden={hidden}
        data-split={split || undefined}
        data-half={half}
      />
      {drop && (
        <div
          className="pane-drop"
          data-side={drop}
          style={{ "--drop": share } as CSSProperties}
          aria-hidden="true"
        />
      )}
      <TerminalSearch />
      {menu && <TerminalPaneMenu menu={menu} onClose={closeMenu} />}
    </>
  );
}

/** The left pane's share of the terminal area: the divider's in a split, else half (a drop there makes a pair). */
const leftShare = (s: HiveState) => (shownSplit(s) ? s.splitPercent / 100 : 0.5);

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

/** The view of the file pane `id` shows, on its `side` in a pair (25.1). */
function FilePane(props: { id: string; side?: "left" | "right"; onMenu: (menu: TabMenu) => void }) {
  const { id, side, onMenu } = props;
  const file = useHive(
    useShallow((s) => {
      const tab = paneFile(s, id);
      return tab ? { worktree: tab.worktree, path: tab.path } : null;
    }),
  );
  if (!file) return null;
  return (
    <FileView
      worktree={file.worktree}
      file={file}
      pane={side}
      onMenu={(event) => {
        event.preventDefault();
        onMenu({ tab: id, x: event.clientX, y: event.clientY });
      }}
    />
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
  // What shows, left to right: the terminals in their host, each file in its view.
  const panes = useHive(useShallow(shownPanes));
  const file = panes.some((pane) => typeof pane === "string");
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
              <FileTab key={fileKey(item)} file={item} onMenu={setMenu} drag={drag(key)} />
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
      {menu &&
        (typeof menu.tab === "number" ? (
          <TerminalTabMenu tab={menu.tab} at={menu} onClose={closeMenu} />
        ) : (
          <FileTabMenu at={menu} onClose={closeMenu} />
        ))}
      <div className="terminal-body" style={{ "--split": percent / 100 } as CSSProperties}>
        {empty && tabs.length === 0 && !file && <EmptyState />}
        {!empty && selected !== null && tabs.length === 0 && !file && (
          <NoTerminals worktree={selected} />
        )}
        <TerminalHost
          hidden={tabs.length === 0 || !panes.some((pane) => typeof pane === "number")}
          half={
            panes.length > 1 && file ? (typeof panes[0] === "number" ? "left" : "right") : undefined
          }
        />
        {panes.map((pane) =>
          typeof pane === "string" ? (
            <FilePane
              key={pane}
              id={pane}
              side={panes.length > 1 ? (pane === panes[0] ? "left" : "right") : undefined}
              onMenu={setMenu}
            />
          ) : null,
        )}
        {panes.length > 1 && <ResizeHandle side="split" />}
      </div>
    </section>
  );
}
