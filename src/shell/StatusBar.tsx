import { WarningCircleIcon } from "@phosphor-icons/react";
import { useEffect } from "react";
import { version } from "../../package.json";
import { currentSpace, openModal, remoteAccount, spaceAccount, useHive } from "../store";
import { transport } from "../transport";
import { usageLines } from "../usage";
import { isMac } from "../window";
import { HostsItem, LOCAL_LABEL } from "./Hosts";
import { loggedIn, useClaudeLogins, useLogIn } from "./SettingsDialog";

/**
 * The current space's Claude account (22.5, D22.15), read-only: the space's terminals get it.
 * Only shown when there is an account besides the default one. Pressed, it opens the space's
 * dialog, where it is changed. An account that never logged in (21.6) carries one small mark
 * (22.6) whose click opens the log-in terminal (`useLogIn`).
 */
function SpaceAccount() {
  const host = useHive((s) => s.host);
  return host === null ? <LocalAccount /> : <RemoteAccount host={host} />;
}

/** This Mac's space's account, from the settings' accounts (22.5). */
function LocalAccount() {
  const { accounts, default_name } = useHive((s) => s.settings.claude);
  const logins = useHive((s) => s.claudeLogins);
  const space = useHive(currentSpace);
  const dir = useHive((s) => spaceAccount(s, currentSpace(s)));
  if (accounts.length === 0 || !space) return null;
  const name = accounts.find((a) => a.config_dir === dir)?.name ?? default_name;
  const title = `Claude account of ${space.name}; change it in the space's settings`;
  return <AccountName name={name} title={title} dir={dir} loggedIn={loggedIn(logins, dir)} />;
}

/**
 * A remote space's account (27.11): its host's own, as that host's service tells it, never this
 * Mac's (#45). Without a `claude` on the host, or with no account of it logged in, it says so.
 * Nothing until the host told.
 */
function RemoteAccount({ host }: { host: string }) {
  const account = useHive(remoteAccount);
  const named = useHive((s) => (s.hostClaude?.logins.length ?? 0) > 1);
  const space = useHive(currentSpace);
  const dir = useHive((s) => spaceAccount(s, currentSpace(s)));
  if (account === null || !space) return null;
  if (account === "no-claude") {
    return (
      <span
        className="space-account-note"
        title={`Claude Code is not installed on ${host}: install it there to run agents`}
      >
        No Claude on {host}
      </span>
    );
  }
  // Only its default account, never logged in: the host has no account yet.
  const name = account.logged_in || named ? account.name : `No Claude account on ${host}`;
  const title = `Claude account of ${space.name} on ${host}; change it in the space's settings`;
  return <AccountName name={name} title={title} dir={dir} loggedIn={account.logged_in} />;
}

/**
 * The account's name, pressed to open the space's dialog, where it is changed. One that never
 * logged in (21.6) carries a small mark (22.6) whose click opens the log-in terminal
 * (`useLogIn`), on the space's host.
 */
function AccountName(props: {
  name: string;
  title: string;
  dir: string | null;
  loggedIn: boolean;
}) {
  const { name, title, dir } = props;
  const logIn = useLogIn(dir);
  return (
    <span className="space-account-wrap">
      <button
        type="button"
        className="ghost space-account"
        title={title}
        onClick={() => openModal("edit-space")}
      >
        {name}
      </button>
      {props.loggedIn ? null : (
        <button
          type="button"
          className="ghost not-logged-in-mark"
          aria-label={`${name} is not logged in: click to log in`}
          title={
            logIn === null
              ? "Not logged in: select a worktree to log in"
              : "Not logged in: click to log in"
          }
          disabled={logIn === null}
          onClick={logIn ?? undefined}
        >
          <WarningCircleIcon size={12} weight="fill" />
        </button>
      )}
    </span>
  );
}

/**
 * Asks a remote host shown (27.11) for its own Claude accounts: once it is linked, whenever
 * another host is shown, and whenever the window comes back to the front (a `/login` finished).
 */
function useHostClaude() {
  const host = useHive((s) => s.host);
  const linked = useHive((s) => s.host !== null && s.links[s.host]?.state === "connected");
  useEffect(() => {
    if (host === null || !linked) return;
    const ask = () => void transport.listClaudeLogins(host);
    ask();
    window.addEventListener("focus", ask);
    return () => window.removeEventListener("focus", ask);
  }, [host, linked]);
}

// The place and connection state (#141), the space's Claude account (22.5) with its session usage
// (12.1), and the app version; never a message (10.3: `Toasts`).
export function StatusBar() {
  useClaudeLogins();
  useHostClaude();
  const connection = useHive((s) => s.connection);
  // The service reports its distribution in `welcome`; until then only "WSL" is known. On
  // macOS, and on Windows in native mode (12.5.4), the service runs natively and reports none.
  const windows = useHive((s) => s.appMode?.mode === "native");
  const distro = connection.status === "connected" ? connection.distro : null;
  const place = windows ? "Windows" : distro ? `WSL: ${distro}` : isMac() ? "This Mac" : "WSL";
  // The service's files could not move into `~/.hive` (22.1): it said why.
  const warning = connection.status === "connected" ? connection.warning : null;
  // With remote hosts (27.8), one item sums up every link, this Mac's too.
  const hosts = useHive((s) => Object.keys(s.links).length > 0);
  return (
    <footer className="statusbar">
      {hosts ? (
        <HostsItem />
      ) : (
        <div
          className="connection"
          data-status={connection.status}
          title={isMac() || windows ? "Service connection" : "WSL connection"}
        >
          <span className="connection-dot" />
          <span>{place}</span>
          <span className="connection-state">{LOCAL_LABEL[connection.status]}</span>
        </div>
      )}
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
 * (21.8). In a remote space (27.11), only for an account of that host's that logged in.
 */
function SessionUsage() {
  const usage = useHive((s) => s.sessionUsage);
  const week = useHive((s) => s.weekUsage);
  const limits = useHive((s) => s.usageLimits);
  const connected = useHive((s) => s.connection.status === "connected");
  const remote = useHive((s) => s.host !== null);
  const account = useHive(remoteAccount);
  if (remote && (account === null || account === "no-claude" || !account.logged_in)) return null;
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
