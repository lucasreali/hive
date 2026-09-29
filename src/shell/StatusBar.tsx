import { version } from "../../package.json";
import { type Connection, useHive } from "../store";
import { isMac } from "../window";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

// The place and connection state (#141), the current account's session usage (12.1) and the
// app version; never a message (10.3: `Toasts`).
export function StatusBar() {
  const connection = useHive((s) => s.connection);
  // The service reports its distribution in `welcome`; until then only "WSL" is known. On
  // macOS the service runs natively and reports none.
  const distro = connection.status === "connected" ? connection.distro : null;
  const place = distro ? `WSL: ${distro}` : isMac() ? "macOS" : "WSL";
  return (
    <footer className="statusbar">
      <div
        className="connection"
        data-status={connection.status}
        title={isMac() ? "Service connection" : "WSL connection"}
      >
        <span className="connection-dot" />
        <span>{place}</span>
        <span className="connection-state">{LABEL[connection.status]}</span>
      </div>
      <SessionUsage />
      <span className="app-version">v{version}</span>
    </footer>
  );
}

/** "Session 42% · resets 14:30": the service sends none once it reset. */
function SessionUsage() {
  const usage = useHive((s) => s.sessionUsage);
  if (!usage) return null;
  const resets = new Date(usage.resets_at * 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  });
  const title = `Current session (5-hour limit): ${usage.used_percentage}% used, resets at ${resets}`;
  return (
    <span className="session-usage" title={title}>
      Session {usage.used_percentage}% · resets {resets}
    </span>
  );
}
