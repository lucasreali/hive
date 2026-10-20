import { ArrowCircleUpIcon, BellIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { spaceName } from "../notify";
import type { Agent } from "../protocol";
import { goToAgent } from "../shortcuts";
import { activateTab, markInboxRead, pendingAgents, unseenPending, useHive } from "../store";
import { isMac, titleBarDoubleClick, watchFullscreen, windowAction } from "../window";
import { requestUpdate } from "./CloseAppDialog";
import {
  HiveIcon,
  MaximizeIcon,
  MinimizeIcon,
  STATE_LABEL,
  StateIcon,
  TerminalIcon,
  WindowCloseIcon,
} from "./icons";
import { elapsed, useNow } from "./Sidebar";
import { ContextMenu } from "./WorktreeMenu";

// On Windows the window has no native decorations; this bar drags it and holds the window
// buttons. On macOS it lies under the native traffic lights (`tauri.macos.conf.json`, centred
// in the bar), which replace the buttons and get room on the left, except in full screen,
// where they are hidden. Its whole empty part drags the window (Tauri's drag region: the bar
// itself and anything inside the brand); its buttons do not.
// ponytail: no "project / worktree" breadcrumb yet; it shows the active tab once tabs exist (1.7).
export function TitleBar() {
  const mac = isMac();
  const [fullscreen, setFullscreen] = useState(false);
  useEffect(() => watchFullscreen(setFullscreen), []);
  const [bar, setBar] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!mac || !bar) return;
    bar.addEventListener("mouseup", doubleClick);
    return () => bar.removeEventListener("mouseup", doubleClick);
  }, [mac, bar]);
  return (
    <header
      ref={setBar}
      className="titlebar"
      data-mac={mac || undefined}
      data-fullscreen={fullscreen || undefined}
      data-tauri-drag-region
    >
      <div className="titlebar-brand" data-tauri-drag-region="deep">
        <HiveIcon />
        <span>Hive</span>
      </div>
      <UpdateButton />
      <PendingBell />
      {!mac && <WindowControls />}
    </header>
  );
}

/**
 * macOS: a double click on the empty bar follows the system's setting (`titleBarDoubleClick`)
 * instead of Tauri's drag region, which always zooms on the second mouse-up: that mouse-up is
 * stopped here, before it reaches Tauri's listener on the document.
 */
function doubleClick(e: MouseEvent) {
  const target = e.target as Element;
  if (e.button !== 0 || e.detail !== 2 || target.closest("button, [role='menu']")) return;
  e.stopPropagation();
  void titleBarDoubleClick();
}

function WindowControls() {
  return (
    <div className="window-controls">
      <button type="button" title="Minimize" onClick={() => windowAction("minimize")}>
        <MinimizeIcon />
      </button>
      <button type="button" title="Maximize" onClick={() => windowAction("toggleMaximize")}>
        <MaximizeIcon />
      </button>
      <button type="button" title="Close" className="close" onClick={() => windowAction("close")}>
        <WindowCloseIcon />
      </button>
    </div>
  );
}

/** A newer release, downloaded at startup (4.19): a click installs it and restarts Hive. */
function UpdateButton() {
  const update = useHive((s) => s.update);
  if (!update) return null;
  return (
    <button
      type="button"
      className="update-button"
      title="Restart Hive to finish the update"
      disabled={update.installing}
      onClick={requestUpdate}
    >
      <ArrowCircleUpIcon size={16} weight="bold" aria-hidden="true" />
      {update.installing ? "Restarting…" : `Restart to update to v${update.version}`}
    </button>
  );
}

/**
 * The bell, with the number of pending agents not seen yet (8.5). A click opens the inbox (6.5)
 * and marks every alert read and the pending agents seen, clearing the badge; an agent counts
 * again on a new alert. F8 (⇧⌘J on macOS) still goes to the next pending agent.
 */
function PendingBell() {
  const count = useHive(unseenPending);
  // The bell itself, once rendered: the inbox hangs under it.
  const [bell, setBell] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const [close] = useState(() => () => setOpen(false));
  const label = count === 0 ? "Notifications" : `${count} pending: notifications`;
  const toggle = () => {
    if (!open) markInboxRead();
    setOpen(!open);
  };
  return (
    <>
      <button
        ref={setBell}
        type="button"
        className="pending-bell"
        data-pending={count > 0}
        title={`${label} (${isMac() ? "⇧⌘J" : "F8"}: next pending)`}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
      >
        <BellIcon size={16} weight={count > 0 ? "fill" : "regular"} aria-hidden="true" />
        {count > 0 && <span className="pending-count">{count}</span>}
      </button>
      {open && bell && <Inbox bell={bell} onClose={close} />}
    </>
  );
}

/**
 * The bell's dropdown, a menu (arrows move, Enter goes to the agent, Esc closes it): the pending
 * agents first, then the alerts raised, the newest first.
 */
function Inbox({ bell, onClose }: { bell: HTMLButtonElement; onClose: () => void }) {
  const pending = useHive(useShallow(pendingAgents));
  const inbox = useHive((s) => s.inbox);
  const agents = useHive((s) => s.agents);
  const tabs = useHive((s) => s.tabs);
  const states = useHive((s) => s.agentStates);
  const titles = useHive((s) => s.agentTitles);
  // Each item names its agent's space when there are several (6.14).
  const spaces = useHive(useShallow((s) => pending.map((a) => spaceName(s, a.id))));
  const now = useNow();
  // Placed once: a new position would move the focus back to the first item on every tick.
  const [at] = useState(() => {
    const box = bell.getBoundingClientRect();
    return { x: box.right, y: box.bottom + 4 };
  });
  const go = (agent: Agent | undefined) => () => {
    onClose();
    if (agent) goToAgent(agent);
  };
  return (
    <ContextMenu at={at} label="Notifications" onClose={onClose} anchor={bell} className="inbox">
      {pending.map((a, i) => {
        const state = states[a.id]?.state ?? "idle";
        return (
          <button key={a.id} type="button" role="menuitem" onClick={go(a)}>
            <StateIcon state={state} />
            <span className="label">{titles[a.id] ?? "Claude"}</span>
            <span className="inbox-meta">
              {[spaces[i], STATE_LABEL[state]].filter(Boolean).join(" · ")}
            </span>
          </button>
        );
      })}
      {pending.length > 0 && inbox.length > 0 && <hr />}
      {inbox.map((item) => {
        if (item.terminal !== undefined) {
          const tab = tabs.find((t) => t.id === item.terminal);
          return (
            <button
              key={item.id}
              type="button"
              role="menuitem"
              disabled={!tab}
              title={tab ? undefined : "This terminal is closed"}
              onClick={() => {
                onClose();
                if (tab) activateTab(tab);
              }}
            >
              <span className="inbox-terminal" data-failed={item.failed || undefined}>
                <TerminalIcon />
              </span>
              <span className="label">{item.text}</span>
              <span className="inbox-meta">{`${elapsed(item.at, now)} ago`}</span>
            </button>
          );
        }
        const agent = item.agent === undefined ? undefined : agents[item.agent];
        return (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={!agent}
            title={agent ? undefined : "This agent has ended"}
            onClick={go(agent)}
          >
            <StateIcon state={item.state ?? "idle"} />
            <span className="label">{item.text}</span>
            <span className="inbox-meta">
              {[item.space, `${elapsed(item.at, now)} ago`].filter(Boolean).join(" · ")}
            </span>
          </button>
        );
      })}
      {pending.length === 0 && inbox.length === 0 && (
        <button type="button" role="menuitem" disabled>
          No notifications
        </button>
      )}
    </ContextMenu>
  );
}
