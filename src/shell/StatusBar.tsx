import { type Connection, setNotice, useHive } from "../store";
import { isMac } from "../window";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

// The "N active agents" count on the right arrives with agent states (2.1).
export function StatusBar() {
  const connection = useHive((s) => s.connection);
  const notice = useHive((s) => s.notice);
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
      {notice && (
        <button type="button" className="notice" title="Dismiss" onClick={() => setNotice(null)}>
          {notice}
        </button>
      )}
    </footer>
  );
}
