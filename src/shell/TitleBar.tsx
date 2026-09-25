import { ArrowCircleUpIcon, BellIcon } from "@phosphor-icons/react";
import { nextPending } from "../shortcuts";
import { pendingAgents, useHive } from "../store";
import { isMac, windowAction } from "../window";
import { requestUpdate } from "./CloseAppDialog";
import { HiveIcon, MaximizeIcon, MinimizeIcon, WindowCloseIcon } from "./icons";

// On Windows the window has no native decorations; this bar drags it and holds the window
// buttons. On macOS it lies under the native traffic lights (`tauri.macos.conf.json`), which
// replace the buttons and get room on the left.
// ponytail: no "project / worktree" breadcrumb yet; it shows the active tab once tabs exist (1.7).
export function TitleBar() {
  const mac = isMac();
  return (
    <header className="titlebar" data-mac={mac || undefined}>
      <div className="titlebar-brand" data-tauri-drag-region>
        <HiveIcon />
        <span>Hive</span>
      </div>
      <UpdateButton />
      <PendingBell />
      {!mac && <WindowControls />}
    </header>
  );
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
