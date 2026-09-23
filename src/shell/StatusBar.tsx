import { type Connection, useHive } from "../store";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

// 1.4 adds the WSL distribution name and the version-mismatch message;
// the "N active agents" count on the right arrives with agent states (2.1).
export function StatusBar() {
  const status = useHive((s) => s.connection.status);
  return (
    <footer className="statusbar">
      <div className="connection" data-status={status} title="WSL connection">
        <span className="connection-dot" />
        <span>WSL</span>
        <span className="connection-state">{LABEL[status]}</span>
      </div>
    </footer>
  );
}
