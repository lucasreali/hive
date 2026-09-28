import { version } from "../../package.json";
import { type Connection, useHive } from "../store";
import { Select } from "../ui/Select";
import { isMac } from "../window";
import { DEFAULT_ACCOUNT, saveSettings } from "./SettingsDialog";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

/**
 * The current Claude account (12.2): new terminals get it, open ones keep theirs. Only shown when
 * there is an account besides the default one. The value "" is the default account.
 */
function AccountSelect() {
  const { accounts, account } = useHive((s) => s.settings.claude);
  if (accounts.length === 0) return null;
  const options = [
    { value: "", label: DEFAULT_ACCOUNT },
    ...accounts.map((a) => ({ value: a.config_dir, label: a.name })),
  ];
  return (
    <Select
      className="account-select"
      aria-label="Claude account"
      value={account ?? ""}
      options={options}
      onChange={(value) =>
        saveSettings((s) => {
          s.claude.account = value || null;
        })
      }
    />
  );
}

// The place and connection state (#141), the Claude account and the app version; never a
// message (10.3: `Toasts`).
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
      <AccountSelect />
      <span className="app-version">v{version}</span>
    </footer>
  );
}
