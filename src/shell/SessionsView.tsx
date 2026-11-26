import {
  ArrowBendUpRightIcon,
  CaretRightIcon,
  CopyIcon,
  DotsThreeIcon,
  FileTextIcon,
  FolderOpenIcon,
  FolderSimpleIcon,
  type Icon,
  PlayIcon,
  TerminalWindowIcon,
  TrashIcon,
  UserSwitchIcon,
} from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { type MouseEvent, useEffect, useRef, useState } from "react";
import { agentKey } from "../hosts";
import type { Session, SessionFile, Settings } from "../protocol";
import { editAuthor, editLine } from "../provenance";
import {
  ago,
  copy,
  locate,
  MOVE_NOTE,
  remove,
  resume,
  sessionName,
  sessionTokens,
} from "../sessions";
import {
  type HiveState,
  onRemote,
  openSessionMenu,
  type SessionFocus,
  showFailure,
  useHive,
} from "../store";
import { transport } from "../transport";
import { ICON, RefreshIcon, StateIcon } from "./icons";
import { leaveFile } from "./RightPanel";
import { SearchField } from "./SearchField";
import { Submenu } from "./TerminalArea";
import { ContextMenu } from "./WorktreeMenu";

/**
 * The name of the Claude account whose folder holds the session's log (the one it last ran as),
 * for its row; null with only the default account.
 */
function accountName(
  { accounts, default_name }: Settings["claude"],
  session: Session,
): string | null {
  if (accounts.length === 0) return null;
  const own = accounts.find((a) => a.config_dir === session.config_dir);
  return own?.name ?? default_name;
}

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
  const truncated = useHive((s) => s.sessionsTruncated);
  const agents = useHive((s) => s.agents);
  // A remote's agents are keyed by host (26.18).
  const host = useHive((s) => s.host);
  const [query, setQuery] = useState("");
  const space = useHive((s) => s.currentSpace);
  // Asked when the tab opens and for another space (the service lists the current space's
  // only); the service sends the list again whenever it changes (D19.4).
  // biome-ignore lint/correctness/useExhaustiveDependencies: `space` says when to ask again.
  useEffect(() => void transport.listSessions(), [space]);
  const q = query.trim().toLowerCase();
  const matches = (x: Session) =>
    [x.title, x.last_text, x.branch, x.id].some((v) => v?.toLowerCase().includes(q));
  const shown = (sessions ?? []).filter((x) => x.worktree === worktree && matches(x));
  // "Who wrote this line?" (27.5): the session it names, scrolled to, or shown on top when this
  // worktree does not list it.
  const focus = useHive((s) => s.sessionFocus);
  const focusAt = focus ? shown.findIndex((x) => x.id === focus.edit.session) : -1;
  // Hundreds of sessions: only the rows in view are rendered, each measured as it shows. A size
  // is kept by session id, not place: the refresh and the search move a session to another index.
  const scroller = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({
    count: shown.length,
    getItemKey: (i) => shown[i].id,
    getScrollElement: () => scroller.current,
    estimateSize: () => 72,
    overscan: 6,
  });
  const scrolled = useRef<SessionFocus | null>(null);
  useEffect(() => {
    if (focusAt < 0 || scrolled.current === focus) return;
    scrolled.current = focus;
    virtual.scrollToIndex(focusAt, { align: "start" });
  }, [focus, focusAt, virtual]);
  return (
    <>
      <div className="files-search sessions-search">
        <SearchField
          aria-label="Search sessions"
          placeholder="Search sessions"
          value={query}
          onChange={setQuery}
          spellCheck={false}
          autoComplete="off"
        />
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
      {focus && !listsFocus(sessions ?? [], worktree, focus) && (
        <FocusFrom focus={focus} worktree={worktree} sessions={sessions ?? []} />
      )}
      <div className="sessions hive-scroll" ref={scroller}>
        <ul aria-label="Sessions" style={{ height: virtual.getTotalSize(), position: "relative" }}>
          {sessions && shown.length === 0 && (
            <li className="hint">No Claude sessions in this worktree.</li>
          )}
          {virtual.getVirtualItems().map((item) => {
            const x = shown[item.index];
            return (
              <SessionRow
                key={x.id}
                session={x}
                live={!!agents[agentKey(host, x.id)]}
                worktree={worktree}
                index={item.index}
                measure={virtual.measureElement}
                top={item.start}
              />
            );
          })}
        </ul>
        {truncated && <div className="hint">Older sessions are not listed.</div>}
      </div>
    </>
  );
}

/** Whether worktree `worktree` lists the session `focus` names (a search may hide it). */
const listsFocus = (sessions: Session[], worktree: string, focus: SessionFocus) =>
  sessions.some((x) => x.id === focus.edit.session && x.worktree === worktree);

/** The name of the worktree `id`, as the tree shows it; its path when not followed. */
const worktreeName = (projects: HiveState["projects"], id: string) =>
  Object.values(projects ?? {})
    .flatMap((p) => p.worktrees)
    .find((w) => w.id === id)?.name ?? id;

/**
 * The session an edit names when the shown worktree does not list it (27.5, Q4): on top, under
 * "From <worktree>", without switching worktree; a deleted one (its record stays) by its name.
 */
function FocusFrom({
  focus,
  worktree,
  sessions,
}: {
  focus: SessionFocus;
  worktree: string;
  sessions: Session[];
}) {
  const { edit } = focus;
  const session = sessions.find((x) => x.id === edit.session);
  const live = useHive((s) => !!session && !!s.agents[agentKey(s.host, session.id)]);
  const from = useHive((s) => worktreeName(s.projects, session?.worktree ?? edit.worktree));
  return (
    <section className="sessions-from" aria-label={`From ${from}`}>
      <h3 className="sessions-from-title">From {from}</h3>
      <ul>
        {session ? (
          <SessionRow session={session} live={live} worktree={worktree} />
        ) : (
          <li className="session" data-focused>
            <div className="session-main">
              <span className="session-title">
                <span className="label">{editAuthor(edit)}</span>
              </span>
              <span className="session-edit">{editLine(edit)}</span>
            </div>
            <SessionFiles id={edit.session} worktree={worktree} focus={focus} />
          </li>
        )}
      </ul>
    </section>
  );
}

interface RowProps {
  session: Session;
  live: boolean;
  /** The worktree the view shows: a file of another one names its worktree. */
  worktree: string;
  /** In the virtual list: its place, how it is measured and where it is put. */
  index?: number;
  measure?: (row: HTMLLIElement | null) => void;
  top?: number;
}

function SessionRow({ session: x, live, worktree, index, measure, top }: RowProps) {
  // A session in a Hive terminal has its live state; any other, the one its log tells.
  const state = useHive((s) =>
    live ? (s.agentStates[agentKey(s.host, x.id)]?.state ?? "idle") : x.state,
  );
  const menu = menuAt(x);
  // The account it last ran as (its log's folder), when there is more than one.
  const account = useHive((s) => accountName(s.settings.claude, x));
  // Running outside Hive: nothing to do here, and nothing said.
  const title = live ? "Show its terminal" : x.running ? undefined : "Resume in its worktree";
  // The edit "Who wrote this line?" found (27.5), and whether its files are listed.
  const focus = useHive((s) => (s.sessionFocus?.edit.session === x.id ? s.sessionFocus : null));
  const open = useHive((s) => s.openSessionFiles.includes(x.id));
  const files = x.files_changed;
  return (
    <li
      className="session"
      data-live={live}
      data-running={x.running}
      data-focused={focus ? true : undefined}
      data-index={index}
      ref={measure}
      style={top === undefined ? undefined : { transform: `translateY(${top}px)` }}
    >
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
          {[`${x.messages} msgs`, sessionTokens(x), ago(x.updated_ms), x.model, account]
            .filter(Boolean)
            .join(" · ")}
        </span>
        {focus && <span className="session-edit">{editLine(focus.edit)}</span>}
      </button>
      {files > 0 && (
        <button
          type="button"
          className="session-files-toggle"
          aria-expanded={open}
          onClick={() => toggleFiles(x.id)}
        >
          <CaretRightIcon className="session-files-caret" size={10} weight="bold" />
          {files === 1 ? "1 file" : `${files} files`}
        </button>
      )}
      {open && <SessionFiles id={x.id} worktree={worktree} focus={focus} />}
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

/** Opens or closes a session's "Files changed" list. */
const toggleFiles = (id: string) =>
  useHive.setState((s) => ({
    openSessionFiles: s.openSessionFiles.includes(id)
      ? s.openSessionFiles.filter((x) => x !== id)
      : [...s.openSessionFiles, id],
  }));

/** A changed file's line: its edits, its lines still the session's, and the last edit's time. */
const fileMeta = (f: SessionFile) =>
  [
    f.edits === 1 ? "1 edit" : `${f.edits} edits`,
    `${f.lines_now} ${f.lines_now === 1 ? "line" : "lines"} now`,
    ago(f.last_ms),
  ].join(" · ");

/**
 * A session's "Files changed" (27.5), asked of the service each time it opens: a click opens
 * the file at the first of its lines that are still the session's. The file an edit was found
 * in is highlighted.
 */
function SessionFiles({
  id,
  worktree,
  focus,
}: {
  id: string;
  worktree: string;
  focus: SessionFocus | null;
}) {
  const listed = useHive((s) => s.sessionFiles[id]);
  useEffect(() => void showFailure(transport.listSessionFiles(id)), [id]);
  const projects = useHive((s) => s.projects);
  if (!listed) return <div className="session-files hint">Loading…</div>;
  return (
    <ul className="session-files" aria-label="Files changed">
      {listed.files.map((f) => {
        const mark = focus?.path === f.path && focus.edit.worktree === f.worktree;
        return (
          <li key={`${f.worktree}\n${f.path}`}>
            <button
              type="button"
              className="session-file"
              data-focused={mark || undefined}
              title={f.path}
              onClick={() =>
                leaveFile({ worktree: f.worktree, path: f.path }, true, f.first_line ?? undefined)
              }
            >
              <span className="session-file-path">{f.path}</span>
              <span className="session-meta">
                {f.worktree === worktree
                  ? fileMeta(f)
                  : `${worktreeName(projects, f.worktree)} · ${fileMeta(f)}`}
              </span>
            </button>
          </li>
        );
      })}
      {listed.truncated && <li className="hint">More files are not listed.</li>}
    </ul>
  );
}

const closeMenu = () => openSessionMenu(null);

/**
 * "Continue in ▸" (21.10): resumes the session as another Claude account, whatever
 * `share_sessions` says, its log moved into that account's folder first. Only with more than
 * one account; not while it runs (its log cannot move).
 */
function ContinueIn({ session, disabled }: { session: Session; disabled: boolean }) {
  const { accounts, default_name } = useHive((s) => s.settings.claude);
  const [open, setOpen] = useState(false);
  if (accounts.length === 0) return null;
  const all = [{ name: default_name, config_dir: null as string | null }, ...accounts];
  const others = all.filter((a) => a.config_dir !== session.config_dir);
  return (
    <div className="submenu-anchor">
      <button
        type="button"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        title={disabled ? "End the session before moving it" : MOVE_NOTE}
        onClick={() => setOpen(!open)}
        onKeyDown={(e) => {
          if (e.key !== "ArrowRight") return;
          e.preventDefault();
          setOpen(true);
        }}
      >
        <UserSwitchIcon {...ICON} />
        Continue in
        <CaretRightIcon className="submenu-caret" {...ICON} />
      </button>
      {open && (
        <Submenu label="Continue in">
          {others.map((a) => (
            <button
              type="button"
              role="menuitem"
              key={a.config_dir ?? ""}
              title={MOVE_NOTE}
              onClick={() => {
                closeMenu();
                void resume(session, false, a.config_dir);
              }}
            >
              {a.name}
            </button>
          ))}
        </Submenu>
      )}
    </div>
  );
}

/** A session's actions (⋯ or right click), as in Orca. */
export function SessionMenu() {
  const menu = useHive((s) => s.sessionMenu);
  const x = useHive((s) => s.sessions?.find((y) => y.id === s.sessionMenu?.session));
  const live = useHive((s) => !!x && !!s.agents[agentKey(s.host, x.id)]);
  const remote = useHive(onRemote);
  if (!menu || !x) return null;
  // Running outside Hive: not resumed or deleted here, with no text.
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
        { disabled: outside },
      )}
      {item(ArrowBendUpRightIcon, "Continue in New Session", () => void resume(x, true))}
      {/* A remote's log does not move to another account (26.18 review): its service is not asked. */}
      {!remote && <ContinueIn session={x} disabled={live || x.running} />}

      {item(
        CopyIcon,
        "Copy Resume Command",
        () => void copy(x.resume_command ?? "", "resume command"),
        x.resume_command === null
          ? { disabled: true, title: "Command Prompt cannot quote this session's folders" }
          : {},
      )}
      {/* A remote's log and folder are not on this Mac (26.18). */}
      {!remote && <hr />}
      {!remote && item(FileTextIcon, "Open Log", () => locate(x, "log", "open"))}
      {!remote && item(FolderOpenIcon, "Reveal Log", () => locate(x, "log", "reveal"))}
      {!remote &&
        item(FolderSimpleIcon, "Open Working Directory", () => locate(x, "folder", "open"))}
      <hr />
      {item(CopyIcon, "Copy Session ID", () => void copy(x.id, "session id"))}
      {item(CopyIcon, "Copy Log Path", () => void copy(x.log, "log path"))}
      <hr />
      {item(TrashIcon, "Delete", () => remove(x), {
        className: "danger",
        disabled: live || outside,
        title: live ? "End the session before deleting it" : undefined,
      })}
    </ContextMenu>
  );
}
