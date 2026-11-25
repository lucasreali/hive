import { type LinkState, linkDown, useHive } from "../store";
import { transport } from "../transport";
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
      <span>Reconnecting to {host}…</span>
    </div>
  );
}
