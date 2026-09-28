import { version } from "../../package.json";
import { type Connection, useHive } from "../store";
import { isMac } from "../window";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

// The place and connection state (#141) and the app version; never a message (10.3: `Toasts`).
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
      <span className="app-version">v{version}</span>
    </footer>
  );
}
