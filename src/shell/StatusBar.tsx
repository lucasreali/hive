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

// The place and connection state (#141), the Claude account (12.2) with its session usage
// (12.1), and the app version; never a message (10.3: `Toasts`).
export function StatusBar() {
  const connection = useHive((s) => s.connection);
  // The service reports its distribution in `welcome`; until then only "WSL" is known. On
  // macOS, and on Windows in native mode (12.5.4), the service runs natively and reports none.
  const windows = useHive((s) => s.appMode?.mode === "native");
  const distro = connection.status === "connected" ? connection.distro : null;
  const place = windows ? "Windows" : distro ? `WSL: ${distro}` : isMac() ? "macOS" : "WSL";
  return (
    <footer className="statusbar">
      <div
        className="connection"
        data-status={connection.status}
        title={isMac() || windows ? "Service connection" : "WSL connection"}
      >
        <span className="connection-dot" />
        <span>{place}</span>
        <span className="connection-state">{LABEL[connection.status]}</span>
      </div>
      <div className="statusbar-account">
        <AccountSelect />
        <SessionUsage />
      </div>
      <span className="app-version">v{version}</span>
    </footer>
  );
}

/** The ring turns the warning colour from this share of the 5-hour window (15.3). */
const USAGE_WARNING = 80;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** "14:30" in local time. */
function clock(at: Date) {
  return at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

/**
 * The session (5-hour) window as a ring filled to its percentage (15.3), with "Session 42% ·
 * resets 14:30" and, when the service sent one, "Week 41% · resets Mon 09:00" as its tooltip
 * (on hover and keyboard focus) and accessible name. The service sends none once it reset.
 */
function SessionUsage() {
  const usage = useHive((s) => s.sessionUsage);
  const week = useHive((s) => s.weekUsage);
  if (!usage) return null;
  const lines = [
    `Session ${usage.used_percentage}% · resets ${clock(new Date(usage.resets_at * 1000))}`,
  ];
  if (week) {
    const at = new Date(week.resets_at * 1000);
    lines.push(`Week ${week.used_percentage}% · resets ${WEEKDAYS[at.getDay()]} ${clock(at)}`);
  }
  return (
    // A button only so the keyboard reaches its tooltip (as the ARIA tooltip pattern); it does
    // nothing when pressed.
    <button
      type="button"
      className="session-usage"
      aria-label={lines.join(", ")}
      data-warning={usage.used_percentage >= USAGE_WARNING || undefined}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <circle className="usage-track" cx="7" cy="7" r="5.5" />
        <circle
          className="usage-arc"
          cx="7"
          cy="7"
          r="5.5"
          pathLength={100}
          strokeDasharray={`${usage.used_percentage} 100`}
          transform="rotate(-90 7 7)"
        />
      </svg>
      <span className="usage-tip" aria-hidden="true">
        {lines.map((line) => (
          <span key={line}>{line}</span>
        ))}
      </span>
    </button>
  );
}
