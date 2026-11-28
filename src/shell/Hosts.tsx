import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { joinedName } from "../protocol";
import {
  type Connection,
  type HiveState,
  hostKey,
  type LinkState,
  linkDown,
  useHive,
} from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";
import { SearchField } from "./SearchField";
import { openSettings, saveSettings } from "./SettingsDialog";
import { useElapsed } from "./Sidebar";

// A remote host's link in the interface (26.18, spike §5; many hosts, 27.8): the status bar's
// one host item and its popover, the new space's host picker, the sidebar's note while the
// shown host is down, the overlay over its terminals, and Settings → Hosts.

/**
 * With more remote hosts than this (27.8), the space switcher filters and collapses its
 * sections, and the status bar's popover, the host picker and Settings → Hosts filter theirs.
 */
export const MANY_HOSTS = 5;

/** A host as the UI names it: this Mac's service is "This Mac". */
export const hostName = (host: string | null) => host ?? "This Mac";

/** Whether `name` matches a filter `query` (case-insensitive; an empty query matches all). */
export const matches = (name: string, query: string) =>
  name.toLowerCase().includes(query.trim().toLowerCase());

/** A link with a problem (27.8): down (reconnecting or disconnected), mismatched or refused. */
export const hostProblem = (link: LinkState | undefined) =>
  link?.state === "reconnecting" || link?.state === "disconnected";

/** A link still coming up: connecting, or installing Hive there. */
const comingUp = (link: LinkState) => link.state === "connecting" || link.state === "installing";

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
      // A joined Hive's version is its owner's to update (29.12): its reason says so.
      return link.kind === "version" && !link.owner
        ? `disconnected: version ${link.version ?? "?"}, this app ${link.app_version ?? "?"}: restart the remote service to update it`
        : `disconnected: ${link.reason}`;
    case "removed":
      return "removed";
  }
}

/** This Mac's service connection, as the status bar says it. */
export const LOCAL_LABEL: Record<Connection["status"], string> = {
  connecting: "connecting",
  connected: "connected",
  version_mismatch: "version mismatch",
  disconnected: "disconnected",
};

/** "reconnecting since 12s", counted as it goes. */
function Since({ since }: { since: number }) {
  return <>{`reconnecting since ${useElapsed(since)}`}</>;
}

/** The dot of a host: this Mac's service connection, or a remote's link (`hostStatus`). */
export function HostDot({ status }: { status: string }) {
  return (
    <span className="connection" data-status={status}>
      <span className="connection-dot" />
    </span>
  );
}

/** The state (`data-status`) of `host`'s dot: this Mac's connection, or the remote's link. */
export const hostStatus = (
  s: Pick<HiveState, "connection" | "links">,
  host: string | null,
): string => (host === null ? s.connection.status : (s.links[host]?.state ?? "connecting"));

/**
 * The status bar's one host item, once a host is added (27.8): "3 hosts", or how many are down
 * ("1 of 3 hosts down", in the warning colour) or still connecting; this Mac's service's state
 * instead while it is not connected. Its dot is the worst link's.
 */
export function hostsSummary(
  local: Connection["status"],
  links: Record<string, LinkState>,
): { text: string; status: string; warning: boolean } {
  if (local !== "connected") {
    const status = local === "connecting" ? "connecting" : "disconnected";
    return { text: `This Mac ${LOCAL_LABEL[local]}`, status, warning: status !== "connecting" };
  }
  const all = Object.values(links);
  const n = all.length;
  const of = (count: number) => (n === 1 ? "1 host" : `${count} of ${n} hosts`);
  const down = all.filter(hostProblem).length;
  if (down > 0) return { text: `${of(down)} down`, status: "disconnected", warning: true };
  const up = all.filter(comingUp).length;
  if (up > 0) return { text: `${of(up)} connecting`, status: "connecting", warning: false };
  return { text: n === 1 ? "1 host" : `${n} hosts`, status: "connected", warning: false };
}

/**
 * The status bar's host item while hosts are added (27.8): their summary (`hostsSummary`); a
 * click opens the popover of every host (`HostsPopover`).
 */
export function HostsItem() {
  const local = useHive((s) => s.connection.status);
  const links = useHive((s) => s.links);
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const summary = hostsSummary(local, links);
  return (
    <>
      <button
        ref={setAnchor}
        type="button"
        className="ghost connection hosts-item"
        data-status={summary.status}
        data-warning={summary.warning || undefined}
        aria-haspopup="dialog"
        aria-expanded={open}
        title="Hosts: their links"
        onClick={() => setOpen(!open)}
      >
        <span className="connection-dot" />
        <span>{summary.text}</span>
      </button>
      {open && anchor && (
        <HostsPopover
          anchor={anchor}
          onClose={(refocus) => {
            setOpen(false);
            if (refocus) anchor.focus();
          }}
        />
      )}
    </>
  );
}

/**
 * Every host with its link (27.8): the ones with a problem first, then the others, this Mac
 * first among them; reconnecting since when, Reconnect while down, "Restart the remote service"
 * for another version (Q8), which one is shown, and a link to Settings → Hosts. A filter with
 * many hosts. Esc or a click outside closes it.
 */
export function HostsPopover({
  anchor,
  onClose,
}: {
  anchor: HTMLElement;
  onClose: (refocus: boolean) => void;
}) {
  const links = useHive((s) => s.links);
  const connection = useHive((s) => s.connection.status);
  const shown = useHive((s) => s.host);
  const [query, setQuery] = useState("");
  const box = useRef<HTMLDivElement>(null);
  // Placed once, over the item: the status bar is at the window's bottom.
  const [at] = useState(() => {
    const r = anchor.getBoundingClientRect();
    return { left: r.left, bottom: window.innerHeight - r.top + 4 };
  });
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const outside = (e: MouseEvent) => {
      const target = e.target as Node;
      if (!box.current?.contains(target) && !anchor.contains(target)) close.current(false);
    };
    // Esc with the focus nowhere (the button pressed went, e.g. Reconnect once linked).
    const onEscape = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") close.current(true);
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", onEscape);
    box.current?.querySelector<HTMLElement>("input, button")?.focus();
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", onEscape);
    };
  }, [anchor]);
  const remotes = Object.keys(links);
  const many = remotes.length > MANY_HOSTS;
  const problem = (host: string | null) =>
    host === null
      ? connection === "version_mismatch" || connection === "disconnected"
      : hostProblem(links[host]);
  const hosts = [null, ...remotes]
    .filter((h) => !many || matches(hostName(h), query))
    .sort((a, b) => Number(problem(b)) - Number(problem(a)));
  return (
    <div
      ref={box}
      className="hosts-popover"
      role="dialog"
      aria-label="Hosts"
      style={{ left: at.left, bottom: at.bottom }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose(true);
          return;
        }
        // ↑/↓ go through the filter and the buttons: WebKit's Tab skips buttons.
        const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
        if (!step) return;
        e.preventDefault();
        const stops = [
          ...e.currentTarget.querySelectorAll<HTMLElement>("input, button:not(.search-clear)"),
        ];
        const at = stops.indexOf(document.activeElement as HTMLElement);
        stops[Math.max(0, Math.min(stops.length - 1, at + step))]?.focus();
      }}
    >
      {many && (
        <SearchField
          aria-label="Filter hosts"
          placeholder="Filter hosts"
          value={query}
          onChange={setQuery}
        />
      )}
      <ul className="hosts-popover-list">
        {hosts.map((host) =>
          host === null ? (
            <li key="" className="hosts-popover-row" data-status={connection}>
              <HostDot status={connection} />
              <span className="hosts-popover-name">This Mac</span>
              <span className="connection-state">{LOCAL_LABEL[connection]}</span>
              {shown === null && <span className="hosts-popover-shown">shown</span>}
            </li>
          ) : (
            <HostRow key={host} host={host} link={links[host]} shown={shown === host} />
          ),
        )}
        {hosts.length === 0 && <li className="hint">No host matches</li>}
      </ul>
      <button
        type="button"
        className="ghost hosts-popover-settings"
        onClick={() => {
          onClose(false);
          openSettings("Hosts");
        }}
      >
        Hosts settings…
      </button>
    </div>
  );
}

/** A remote host's row in the popover: its link and, while it is down, its action. */
function HostRow({ host, link, shown }: { host: string; link: LinkState; shown: boolean }) {
  return (
    <li className="hosts-popover-row" data-status={link.state}>
      <HostDot status={link.state} />
      <span className="hosts-popover-name">{host}</span>
      <span
        className="connection-state"
        title={link.state === "reconnecting" ? link.reason : linkText(link)}
      >
        {link.state === "reconnecting" ? <Since since={link.since} /> : linkText(link)}
      </span>
      {shown && <span className="hosts-popover-shown">shown</span>}
      <HostActions host={host} link={link} />
    </li>
  );
}

/**
 * What a host's link offers while it has a problem (27.8, 27.10): Reconnect while it is down or
 * refused, "Restart the remote service" for another version (Q8), which ends its kept terminals.
 * The status bar's popover and Settings → Hosts both show it.
 */
export function HostActions({ host, link }: { host: string; link: LinkState | undefined }) {
  if (!link || !hostProblem(link)) return null;
  // Only the owner restarts a joined Hive (29.12, Q5): its link offers Reconnect only.
  if (link.state === "disconnected" && link.kind === "version" && !link.owner) {
    const terminals = link.terminals || 0;
    return (
      <button
        type="button"
        className="ghost host-action"
        aria-label={`Restart the remote service on ${host}`}
        title={`Ends its ${terminals === 1 ? "terminal" : `${terminals} terminals`}`}
        onClick={() => void transport.restartRemote(host)}
      >
        Restart the remote service
      </button>
    );
  }
  return (
    <button
      type="button"
      className="ghost host-action"
      aria-label={`Reconnect ${host}`}
      onClick={() => void transport.reconnectHost(host)}
    >
      Reconnect
    </button>
  );
}

/**
 * Where a new space goes (27.8): this Mac or a host, picked from a list (↑/↓ or a click), with a
 * filter when there are many hosts. A host whose link is down cannot take one.
 */
export function HostPicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (host: string | null) => void;
}) {
  const links = useHive((s) => s.links);
  const [query, setQuery] = useState("");
  const remotes = Object.keys(links);
  const many = remotes.length > MANY_HOSTS;
  const hosts = [null, ...remotes].filter((h) => !many || matches(hostName(h), query));
  const usable = hosts.filter((h) => !linkDown({ links }, h));
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (!step) return;
    e.preventDefault();
    const at = usable.indexOf(value);
    const next = usable[Math.max(0, Math.min(usable.length - 1, at + step))];
    if (next !== undefined && next !== value) onChange(next);
  };
  return (
    <div className="field">
      <span id="space-host-label">Where</span>
      {many && (
        <SearchField
          aria-label="Filter hosts"
          placeholder="Filter hosts"
          value={query}
          onChange={setQuery}
          onKeyDown={onKeyDown}
        />
      )}
      <div
        className="host-picker"
        role="listbox"
        tabIndex={0}
        aria-labelledby="space-host-label"
        aria-activedescendant={`space-host-${hostKey(value)}`}
        onKeyDown={onKeyDown}
      >
        {hosts.map((host) => {
          const down = linkDown({ links }, host);
          return (
            // biome-ignore lint/a11y/useKeyWithClickEvents: the listbox handles the keys.
            <div
              key={hostKey(host)}
              id={`space-host-${hostKey(host)}`}
              role="option"
              tabIndex={-1}
              aria-selected={host === value}
              aria-disabled={down || undefined}
              title={down && host !== null ? `${host}: ${linkText(links[host])}` : undefined}
              onClick={() => {
                if (!down && host !== value) onChange(host);
              }}
            >
              <HostDot status={host === null ? "connected" : links[host].state} />
              <span>{hostName(host)}</span>
            </div>
          );
        })}
        {hosts.length === 0 && <div className="hint">No host matches</div>}
      </div>
    </div>
  );
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
 * With many hosts (27.8) the list scrolls and a filter narrows it. A link with a problem offers
 * the popover's action (27.10, `HostActions`): Reconnect, or "Restart the remote service".
 */
export function HostsSettings() {
  const own = useHive((s) => s.settings.remote.hosts);
  const joined = useHive((s) => s.settings.remote.joined);
  const links = useHive((s) => s.links);
  const [alias, setAlias] = useState("");
  const [query, setQuery] = useState("");
  // The user's own hosts, then the Hives joined (29.12), by the names their links have.
  const hosts = [...own, ...joined.map(joinedName)];
  const many = hosts.length > MANY_HOSTS;
  const listed = many ? hosts.filter((h) => matches(h, query)) : hosts;
  const remove = (host: string) =>
    saveSettings((s) => {
      s.remote.hosts = s.remote.hosts.filter((h) => h !== host);
      s.remote.joined = s.remote.joined.filter((j) => joinedName(j) !== host);
    });
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
      {many && (
        <SearchField
          aria-label="Filter hosts"
          placeholder="Filter hosts"
          value={query}
          onChange={setQuery}
        />
      )}
      <div className="hosts" data-many={many || undefined}>
        {listed.map((host) => {
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
              <span className="host-row-actions">
                <HostActions host={host} link={link} />
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
                onClick={() => remove(host)}
              >
                <CloseIcon />
              </button>
            </div>
          );
        })}
        {listed.length === 0 && many && <p className="field-help">No host matches.</p>}
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
      <JoinHive />
    </section>
  );
}

/**
 * "Join someone's Hive" (29.12): the alias of your own login on their host, and their login
 * name there. The link runs your own `hive bridge --join <owner>` on that host; the service
 * checks both when the settings are saved.
 */
export function JoinHive() {
  const [alias, setAlias] = useState("");
  const [owner, setOwner] = useState("");
  const ready = alias.trim() !== "" && owner.trim() !== "";
  return (
    <form
      className="host-add host-join"
      aria-labelledby="join-title"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ready) return;
        const joined = { alias: alias.trim(), owner: owner.trim() };
        saveSettings((s) => {
          s.remote.joined.push(joined);
        });
        setAlias("");
        setOwner("");
      }}
    >
      <h4 id="join-title">Join someone's Hive</h4>
      <p className="field-help">
        A teammate shares their Hive on a host you log in to as yourself (their Settings → Sharing).
        It runs as them; you see the spaces they share.
      </p>
      <input
        aria-label="Your alias of their host"
        placeholder="Alias in ~/.ssh/config"
        value={alias}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        onChange={(e) => setAlias(e.currentTarget.value)}
      />
      <input
        aria-label="Their login name"
        placeholder="Their login name"
        value={owner}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        onChange={(e) => setOwner(e.currentTarget.value)}
      />
      <button type="submit" className="secondary" disabled={!ready}>
        Join
      </button>
    </form>
  );
}
