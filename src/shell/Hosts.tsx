import { useState } from "react";
import { type LinkState, linkDown, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";
import { saveSettings } from "./SettingsDialog";
import { useElapsed } from "./Sidebar";

// A remote host's link in the interface (26.18, spike §5): its chip in the status bar, the
// sidebar's note while the shown host is down, and the overlay over its terminals.

/** What a link's chip says after the host's name, but for `reconnecting` (`Since`). */
export function linkText(link: LinkState): string {
  switch (link.state) {
    case "connecting":
      return "connecting";
    case "installing":
      return `installing Hive (${link.cpu})`;
    case "connected":
      return "connected";
    case "reconnecting":
      return "reconnecting";
    case "disconnected":
      return link.kind === "version"
        ? `disconnected: version ${link.version ?? "?"}, this app ${link.app_version ?? "?"}`
        : `disconnected: ${link.reason}`;
    case "removed":
      return "removed";
  }
}

/** "reconnecting since 12s", counted as it goes. */
function Since({ since }: { since: number }) {
  return <>{`reconnecting since ${useElapsed(since)}`}</>;
}

/**
 * The status bar's chip of each remote host with a link (26.18): connected, reconnecting since
 * when, or disconnected and why, with Reconnect while it is down. A remote service of another
 * version (Q8) offers to restart it, which ends its terminals, or to stay disconnected.
 */
export function HostChips() {
  const links = useHive((s) => s.links);
  return Object.entries(links).map(([host, link]) => {
    const down = link.state === "reconnecting" || link.state === "disconnected";
    const version = link.state === "disconnected" && link.kind === "version";
    const terminals = (version && link.terminals) || 0;
    return (
      <span key={host} className="connection host-chip" data-status={link.state}>
        <span className="connection-dot" />
        <span>{host}</span>
        <span
          className="connection-state"
          title={link.state === "reconnecting" ? link.reason : linkText(link)}
        >
          {link.state === "reconnecting" ? <Since since={link.since} /> : linkText(link)}
        </span>
        {down && !version && (
          <button
            type="button"
            className="ghost host-action"
            onClick={() => void transport.reconnectHost(host)}
          >
            Reconnect
          </button>
        )}
        {version && (
          <button
            type="button"
            className="ghost host-action"
            title={`Ends its ${terminals === 1 ? "terminal" : `${terminals} terminals`}`}
            onClick={() => void transport.restartRemote(host)}
          >
            Restart the remote service
          </button>
        )}
      </span>
    );
  });
}

/**
 * Over the sidebar's tree while the shown host's link is down (spike §5): the last tree stays,
 * dimmed, under "Reconnecting to gpu-box…" and a Reconnect button.
 */
export function HostDown() {
  const host = useHive((s) => (linkDown(s, s.host) ? s.host : null));
  if (host === null) return null;
  return (
    <div className="host-down" role="status">
      <span>Reconnecting to {host}…</span>
      <button
        type="button"
        className="ghost host-action"
        onClick={() => void transport.reconnectHost(host)}
      >
        Reconnect
      </button>
    </div>
  );
}

/**
 * Over the terminals shown while their host's link is down (26.18): they are all the shown
 * host's. They take no input (`terminals.ts`) and keep their buffers; the output they missed
 * comes when the link is back (26.16).
 */
export function HostOverlay() {
  const host = useHive((s) => (linkDown(s, s.host) ? s.host : null));
  if (host === null) return null;
  return (
    <div className="host-overlay" role="status">
      <span className="host-overlay-text">Reconnecting to {host}…</span>
    </div>
  );
}

/**
 * Settings → Hosts (26.18): the remote hosts the app links to, each by its alias in
 * `~/.ssh/config`, typed in (Hive does not read that file); each with its link's state, Test
 * (tries the link now) and Remove. The service checks an alias when the settings are saved.
 */
export function HostsSettings() {
  const hosts = useHive((s) => s.settings.remote.hosts);
  const links = useHive((s) => s.links);
  const [alias, setAlias] = useState("");
  const add = () => {
    const name = alias.trim();
    if (!name) return;
    saveSettings((s) => {
      s.remote.hosts.push(name);
    });
    setAlias("");
  };
  return (
    <section className="field settings-group" aria-labelledby="hosts-title">
      <h3 id="hosts-title">Remote hosts</h3>
      <p className="field-help">
        Linux hosts reached with your own <code>ssh</code>, by their alias in{" "}
        <code>~/.ssh/config</code>. Hive runs its service there; each host keeps its own spaces.
      </p>
      <div className="hosts">
        {hosts.map((host) => {
          const link = links[host];
          return (
            <div className="host-row" key={host}>
              <span className="connection" data-status={link?.state ?? "connecting"}>
                <span className="connection-dot" />
                <span>{host}</span>
              </span>
              <span className="connection-state host-row-state" title={link && linkText(link)}>
                {link ? linkText(link) : "not linked yet"}
              </span>
              <button
                type="button"
                className="secondary"
                title={`Try the link to ${host} now`}
                onClick={() => void transport.reconnectHost(host)}
              >
                Test
              </button>
              <button
                type="button"
                className="ghost"
                title={`Remove ${host}`}
                aria-label={`Remove ${host}`}
                onClick={() =>
                  saveSettings((s) => {
                    s.remote.hosts = s.remote.hosts.filter((h) => h !== host);
                  })
                }
              >
                <CloseIcon />
              </button>
            </div>
          );
        })}
      </div>
      <form
        className="host-add"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          aria-label="Host alias"
          placeholder="Alias in ~/.ssh/config"
          value={alias}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          onChange={(e) => setAlias(e.currentTarget.value)}
        />
        <button type="submit" className="secondary" disabled={!alias.trim()}>
          Add host
        </button>
      </form>
    </section>
  );
}
