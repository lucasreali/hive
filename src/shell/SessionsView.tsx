import { ArrowClockwiseIcon, DotsThreeIcon } from "@phosphor-icons/react";
import { type MouseEvent, useEffect, useState } from "react";
import { ago, copy, locate, remove, resume, resumeCommand, sessionName } from "../sessions";
import { openSessionMenu, type Session, useHive } from "../store";
import { transport } from "../transport";
import { StateIcon } from "./icons";
import { ContextMenu } from "./WorktreeMenu";

/** Opens the session's menu at the pointer, or under the ⋯ button that was clicked. */
function menuAt(session: Session) {
  return (event: MouseEvent<HTMLElement>) => {
    event.preventDefault();
    const box = event.currentTarget.getBoundingClientRect();
    const pointer = event.type === "contextmenu" && (event.clientX || event.clientY);
    const x = pointer ? event.clientX : box.left;
    const y = pointer ? event.clientY : box.bottom;
    openSessionMenu({ session: session.id, x, y });
  };
}

/**
 * Sessions: Claude Code's sessions of the shown worktree, read by the service from their logs,
 * the most recent first; the search matches titles, last messages, branches and ids. A click
 * goes on with the session (its terminal when it runs); ⋯ or a right click has the rest.
 */
export function SessionsView({ worktree }: { worktree: string }) {
  const sessions = useHive((s) => s.sessions);
  const error = useHive((s) => s.sessionsError);
  const agents = useHive((s) => s.agents);
  const [query, setQuery] = useState("");
  useEffect(() => void transport.listSessions(), []);
  const q = query.trim().toLowerCase();
  const matches = (x: Session) =>
    [x.title, x.last_text, x.branch, x.id].some((v) => v?.toLowerCase().includes(q));
  const shown = (sessions ?? []).filter((x) => x.worktree === worktree && matches(x));
  return (
    <>
      <div className="files-search sessions-search">
        <label className="files-search-field">
          <input
            type="search"
            aria-label="Search sessions"
            placeholder="Search sessions"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <span className="sessions-count">{sessions ? `${shown.length} shown` : "Loading…"}</span>
        <button
          type="button"
          className="ghost"
          title="Refresh sessions"
          aria-label="Refresh sessions"
          onClick={() => void transport.listSessions()}
        >
          <ArrowClockwiseIcon size={14} aria-hidden="true" />
        </button>
      </div>
      {error && <div className="files-error">{error}</div>}
      <ul className="sessions hive-scroll" aria-label="Sessions">
        {sessions && shown.length === 0 && (
          <li className="hint">No Claude sessions in this worktree.</li>
        )}
        {shown.map((x) => (
          <SessionRow key={x.id} session={x} live={!!agents[x.id]} />
        ))}
      </ul>
    </>
  );
}

function SessionRow({ session: x, live }: { session: Session; live: boolean }) {
  const state = useHive((s) => s.agentStates[x.id]?.state ?? "idle");
  const menu = menuAt(x);
  return (
    <li className="session" data-live={live}>
      <button
        type="button"
        className="session-main"
        title={live ? "Show its terminal" : "Resume in its worktree"}
        onClick={() => void resume(x)}
        onContextMenu={menu}
      >
        <span className="session-title">
          {live && <StateIcon state={state} />}
          <span className="label">{x.title ?? "Untitled session"}</span>
        </span>
        {x.last_text && (
          <span className="session-last">
            <b>{x.last_role === "user" ? "You" : "Agent"}:</b> {x.last_text}
          </span>
        )}
        <span className="session-meta">
          {[`${x.messages} msgs`, ago(x.updated_ms), x.model].filter(Boolean).join(" · ")}
        </span>
      </button>
      <button
        type="button"
        className="session-more"
        aria-label={`Actions for ${sessionName(x)}`}
        title="Actions"
        onClick={menu}
      >
        <DotsThreeIcon size={16} weight="bold" aria-hidden="true" />
      </button>
    </li>
  );
}

const closeMenu = () => openSessionMenu(null);

/** A session's actions (⋯ or right click), as in Orca. */
export function SessionMenu() {
  const menu = useHive((s) => s.sessionMenu);
  const x = useHive((s) => s.sessions?.find((y) => y.id === s.sessionMenu?.session));
  const live = useHive((s) => !!x && !!s.agents[x.id]);
  if (!menu || !x) return null;
  const item = (label: string, action: () => void, extra: object = {}) => (
    <button
      type="button"
      role="menuitem"
      onClick={() => {
        closeMenu();
        action();
      }}
      {...extra}
    >
      {label}
    </button>
  );
  return (
    <ContextMenu at={menu} label={`Session ${sessionName(x)}`} onClose={closeMenu}>
      {item(live ? "Show Its Terminal" : "Resume in Worktree", () => void resume(x))}
      {item("Continue in New Session", () => void resume(x, true))}
      {item("Copy Resume Command", () => void copy(resumeCommand(x), "resume command"))}
      <hr />
      {item("Open Log", () => locate(x, "log", "open"))}
      {item("Reveal Log", () => locate(x, "log", "reveal"))}
      {item("Open Working Directory", () => locate(x, "folder", "open"))}
      <hr />
      {item("Copy Session ID", () => void copy(x.id, "session id"))}
      {item("Copy Log Path", () => void copy(x.log, "log path"))}
      <hr />
      {item("Delete", () => remove(x), {
        className: "danger",
        disabled: live,
        title: live ? "End the session before deleting it" : undefined,
      })}
    </ContextMenu>
  );
}
