import { BellIcon } from "@phosphor-icons/react";
import { nextPending } from "../shortcuts";
import { pendingAgents, useHive } from "../store";
import { windowAction } from "../window";
import { HiveIcon, MaximizeIcon, MinimizeIcon, WindowCloseIcon } from "./icons";

// The window has no native decorations; this bar drags it and holds the window buttons.
// ponytail: no "project / worktree" breadcrumb yet; it shows the active tab once tabs exist (1.7).
export function TitleBar() {
  return (
    <header className="titlebar">
      <div className="titlebar-brand" data-tauri-drag-region>
        <HiveIcon />
        <span>Hive</span>
      </div>
      <PendingBell />
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
    </header>
  );
}

/**
 * The agents that need you ("N pending", the service's), as a bell with their number; a click
 * is F8: the next pending agent.
 */
function PendingBell() {
  const count = useHive((s) => pendingAgents(s).length);
  const label = count === 0 ? "Nothing pending" : `${count} pending: go to the next (F8)`;
  return (
    <button
      type="button"
      className="pending-bell"
      data-pending={count > 0}
      title={label}
      aria-label={label}
      disabled={count === 0}
      onClick={nextPending}
    >
      <BellIcon size={16} weight={count > 0 ? "fill" : "regular"} aria-hidden="true" />
      {count > 0 && <span className="pending-count">{count}</span>}
    </button>
  );
}
