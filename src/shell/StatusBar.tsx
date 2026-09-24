import { type Connection, setNotice, useHive } from "../store";

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
  // The service reports its distribution in `welcome`; until then only "WSL" is known.
  const distro = connection.status === "connected" ? connection.distro : null;
  return (
    <footer className="statusbar">
      <div className="connection" data-status={connection.status} title="WSL connection">
        <span className="connection-dot" />
        <span>{distro ? `WSL: ${distro}` : "WSL"}</span>
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
