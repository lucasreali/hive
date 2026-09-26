import {
  ArrowBendUpRightIcon,
  ChatTeardropTextIcon,
  CopyIcon,
  DotsThreeIcon,
  FileTextIcon,
  FolderOpenIcon,
  FolderSimpleIcon,
  type Icon,
  PlayIcon,
  TerminalWindowIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { type MouseEvent, useEffect, useState } from "react";
import {
  ago,
  copy,
  locate,
  OUTSIDE,
  openAsChat,
  remove,
  resume,
  resumeCommand,
  sessionName,
  sessionTokens,
} from "../sessions";
import { openSessionMenu, type Session, useHive } from "../store";
import { transport } from "../transport";
import { ICON, RefreshIcon, StateIcon } from "./icons";
import { ContextMenu } from "./WorktreeMenu";

/** How often the open Sessions tab asks for the sessions again. */
export const REFRESH_MS = 5000;

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
  const space = useHive((s) => s.currentSpace);
  // Asked again every few seconds (sessions outside Hive change state without hooks) and for
  // another space: the service lists the current space's only.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new space is listed again.
  useEffect(() => {
    void transport.listSessions();
    const every = setInterval(() => void transport.listSessions(), REFRESH_MS);
    return () => clearInterval(every);
  }, [space]);
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
          <RefreshIcon />
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
  // A session in a Hive terminal has its live state; any other, the one its log tells.
  const state = useHive((s) => (live ? (s.agentStates[x.id]?.state ?? "idle") : x.state));
  const menu = menuAt(x);
  const title = live ? "Show its terminal" : x.running ? OUTSIDE : "Resume in its worktree";
  return (
    <li className="session" data-live={live} data-running={x.running}>
      <button
        type="button"
        className="session-main"
        title={title}
        onClick={() => void resume(x)}
        onContextMenu={menu}
      >
        <span className="session-title">
          <StateIcon state={state} />
          <span className="label">{x.title ?? "Untitled session"}</span>
        </span>
        {x.last_text && (
          <span className="session-last">
            <b>{x.last_role === "user" ? "You" : "Agent"}:</b> {x.last_text}
          </span>
        )}
        <span className="session-meta">
          {[`${x.messages} msgs`, sessionTokens(x), ago(x.updated_ms), x.model]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </button>
      <button
        type="button"
        className="session-more"
        aria-label={`Actions for ${sessionName(x)}`}
        title="Actions"
        onClick={menu}
      >
        <DotsThreeIcon size={18} weight="bold" aria-hidden="true" />
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
  const outside = x.running && !live;
  const item = (Shape: Icon, label: string, action: () => void, extra: object = {}) => (
    <button
      type="button"
      role="menuitem"
      onClick={() => {
        closeMenu();
        action();
      }}
      {...extra}
    >
      <Shape {...ICON} />
      {label}
    </button>
  );
  return (
    <ContextMenu at={menu} label={`Session ${sessionName(x)}`} onClose={closeMenu}>
      {item(
        live ? TerminalWindowIcon : PlayIcon,
        live ? "Show Its Terminal" : "Resume in Worktree",
        () => void resume(x),
        {
          disabled: outside,
          title: outside ? OUTSIDE : undefined,
        },
      )}
      {item(ChatTeardropTextIcon, "Open as Chat", () => void openAsChat(x), {
        disabled: live || x.running,
        title: live || x.running ? "End the session before opening it as a chat" : undefined,
      })}
      {item(ArrowBendUpRightIcon, "Continue in New Session", () => void resume(x, true))}
      {item(CopyIcon, "Copy Resume Command", () => void copy(resumeCommand(x), "resume command"))}
      <hr />
      {item(FileTextIcon, "Open Log", () => locate(x, "log", "open"))}
      {item(FolderOpenIcon, "Reveal Log", () => locate(x, "log", "reveal"))}
      {item(FolderSimpleIcon, "Open Working Directory", () => locate(x, "folder", "open"))}
      <hr />
      {item(CopyIcon, "Copy Session ID", () => void copy(x.id, "session id"))}
      {item(CopyIcon, "Copy Log Path", () => void copy(x.log, "log path"))}
      <hr />
      {item(TrashIcon, "Delete", () => remove(x), {
        className: "danger",
        disabled: live || x.running,
        title: live || x.running ? "End the session before deleting it" : undefined,
      })}
    </ContextMenu>
  );
}
