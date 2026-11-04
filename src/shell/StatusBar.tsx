import { version } from "../../package.json";
import { type Connection, currentSpace, openModal, spaceAccount, useHive } from "../store";
import { usageLines } from "../usage";
import { isMac } from "../window";
import { LogIn, loggedIn, NotLoggedIn, useClaudeLogins } from "./SettingsDialog";

const LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

/**
 * The current space's Claude account (22.5, D22.15), read-only: the space's terminals get it.
 * Only shown when there is an account besides the default one. Pressed, it opens the space's
 * dialog, where it is changed. An account that never logged in says so (21.6), with "Log in…".
 */
function SpaceAccount() {
  const { accounts, default_name } = useHive((s) => s.settings.claude);
  const logins = useHive((s) => s.claudeLogins);
  const space = useHive(currentSpace);
  const dir = useHive((s) => spaceAccount(s, currentSpace(s)));
  if (accounts.length === 0 || !space) return null;
  const name = accounts.find((a) => a.config_dir === dir)?.name ?? default_name;
  return (
    <>
      {loggedIn(logins, dir) ? null : (
        <>
          <NotLoggedIn dir={dir} />
          <LogIn dir={dir} name={name} />
        </>
      )}
      <button
        type="button"
        className="ghost space-account"
        title={`Claude account of ${space.name}; change it in the space's settings`}
        onClick={() => openModal("edit-space")}
      >
        {name}
      </button>
    </>
  );
}

// The place and connection state (#141), the space's Claude account (22.5) with its session usage
// (12.1), and the app version; never a message (10.3: `Toasts`).
export function StatusBar() {
  useClaudeLogins();
  const connection = useHive((s) => s.connection);
  // The service reports its distribution in `welcome`; until then only "WSL" is known. On
  // macOS, and on Windows in native mode (12.5.4), the service runs natively and reports none.
  const windows = useHive((s) => s.appMode?.mode === "native");
  const distro = connection.status === "connected" ? connection.distro : null;
  const place = windows ? "Windows" : distro ? `WSL: ${distro}` : isMac() ? "macOS" : "WSL";
  // The service's files could not move into `~/.hive` (22.1): it said why.
  const warning = connection.status === "connected" ? connection.warning : null;
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
      {warning ? (
        <span className="folder-warning" title={warning}>
          Old folders in use
        </span>
      ) : null}
      <div className="statusbar-account">
        <SpaceAccount />
        <SessionUsage />
      </div>
      <span className="app-version">v{version}</span>
    </footer>
  );
}

/** The ring turns the warning colour from this share of the 5-hour window (15.3). */
const USAGE_WARNING = 80;

/** The empty ring's tooltip: no window for the current account yet (21.8). */
const NO_USAGE = "No usage yet: it appears once an agent of this account answers";

/**
 * The session (5-hour) window as a ring filled to its percentage (15.3), with "Session 42% ·
 * resets 14:30" and, when the service sent one, "Week 41% · resets Mon 09:00" as its tooltip
 * (on hover and keyboard focus) and accessible name, each followed by its pace when the service
 * forecasts a limit before the reset (17.23, `usageLines`, the menu bar icon's too). With none
 * (none came yet, or it reset), a dim empty ring that says so while the service is connected
 * (21.8).
 */
function SessionUsage() {
  const usage = useHive((s) => s.sessionUsage);
  const week = useHive((s) => s.weekUsage);
  const limits = useHive((s) => s.usageLimits);
  const connected = useHive((s) => s.connection.status === "connected");
  if (!usage && !connected) return null;
  const lines = usage ? usageLines(usage, week, limits) : [NO_USAGE];
  return (
    // A button only so the keyboard reaches its tooltip (as the ARIA tooltip pattern); it does
    // nothing when pressed.
    <button
      type="button"
      className="session-usage"
      aria-label={lines.join(", ")}
      data-empty={!usage || undefined}
      data-warning={(usage && usage.used_percentage >= USAGE_WARNING) || undefined}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <circle className="usage-track" cx="7" cy="7" r="5.5" />
        <circle
          className="usage-arc"
          cx="7"
          cy="7"
          r="5.5"
          pathLength={100}
          strokeDasharray={`${usage?.used_percentage ?? 0} 100`}
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
