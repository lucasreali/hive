import { isTauri } from "@tauri-apps/api/core";
import type { OpenSession, ServiceMessage, Session, SessionTarget } from "./protocol";
import { activateTab, ask, currentSpace, showNotice, spaceAccount, useHive } from "./store";
import { openClaude } from "./terminals";
import { transport } from "./transport";
import { openPath } from "./viewer/external";

// What the sidebar's Sessions do with a Claude session. Claude runs in a Hive terminal as the
// user would run it; logs and folders open with Windows' own apps.

type Located = Extract<ServiceMessage, { type: "session_located" }>;
type Handing = "open" | "reveal";

/** How each located path goes to Windows, by `<id>:<target>`, until the service answers. */
const pending = new Map<string, Handing>();

/** `claude`'s arguments to go on with a session, or to start a new one from it (`fork`). */
export const resumeArgs = (session: Session, fork = false) =>
  `--resume ${session.id}${fork ? " --fork-session" : ""}`;

type Moved = Extract<ServiceMessage, { type: "session_moved" | "move_session_failed" }>;

/** The moves asked of the service, by session id, until it answers: why one failed, or null. */
const moving = new Map<string, (error: string | null) => void>();

/** What a rewind loses when a session moves (its `file-history` stays behind, 21.10). */
export const MOVE_NOTE =
  "Its log moves into that account's folder: rewind checkpoints from before the move are lost.";

/**
 * The Claude account a session goes on as: `account` when one is asked ("Continue in"), its
 * project's space's when sessions are shared (21.10, D22.5; the history lists only the current
 * space's projects, so the current space's, 22.5), else the one it ran as (12.2).
 */
export function accountFor(session: Session, account?: string | null): string | null {
  if (account !== undefined) return account;
  const s = useHive.getState();
  return s.settings.claude.share_sessions ? spaceAccount(s, currentSpace(s)) : session.config_dir;
}

/** How long a move may wait for the service's answer. */
export const MOVE_WAIT_MS = 30_000;

/**
 * Asks the service to move a session's log into `account`'s folder; why it could not, or null.
 * One move of a session at a time; no answer within `MOVE_WAIT_MS` ends the wait as a failure.
 */
const moveTo = (id: string, account: string | null) =>
  new Promise<string | null>((done) => {
    if (moving.has(id)) return done("it is already being moved");
    const timer = setTimeout(() => {
      moving.delete(id);
      done("the service did not answer");
    }, MOVE_WAIT_MS);
    moving.set(id, (error) => {
      clearTimeout(timer);
      done(error);
    });
    void transport.moveSession(id, account);
  });

/** The service's answer to a move: the waiting `resume` goes on, or says why not. */
export function onMoved(moved: Moved): void {
  const done = moving.get(moved.id);
  moving.delete(moved.id);
  done?.(moved.type === "session_moved" ? null : moved.message);
}

/**
 * Goes on with the session: shows its terminal when it runs in one, else runs
 * `claude --resume` in a new terminal in its folder, as the account `accountFor` picks. Its log
 * is moved into that account's folder first when it is in another (Claude Code resumes only a
 * log in its own folder). `fork` always starts a new session; one running elsewhere is forked
 * as its own account, since its log cannot move.
 */
export async function resume(
  session: Session,
  fork = false,
  account?: string | null,
): Promise<void> {
  const s = useHive.getState();
  const agent = s.agents[session.id];
  const tab = agent && s.tabs.find((t) => t.id === agent.terminal);
  if (tab && !fork) return activateTab(tab);
  // Running outside Hive: a second `claude` on the same session would write the same log.
  if (session.running && !fork) return;
  const dir = session.running ? session.config_dir : accountFor(session, account);
  if (dir !== session.config_dir) {
    const error = await moveTo(session.id, dir);
    if (error) return showNotice("error", `Cannot move the session: ${error}`);
  }
  try {
    await openClaude(session.cwd, resumeArgs(session, fork), { config_dir: dir });
  } catch (error) {
    showNotice("error", `Cannot open a terminal in ${session.cwd}: ${error}`);
  }
}

/**
 * The sessions that ran in Hive's terminals when the app last closed: each is resumed in a new
 * terminal in its folder, as its terminal's Claude account, one after the other.
 */
export async function restore(sessions: OpenSession[]): Promise<void> {
  for (const { id, cwd, config_dir } of sessions) {
    try {
      await openClaude(cwd, `--resume ${id}`, { config_dir });
    } catch (error) {
      showNotice("error", `Cannot resume the session in ${cwd}: ${error}`);
    }
  }
}

/** Asks the service where Windows sees the session's log or folder, to open or reveal it. */
export function locate(session: Session, target: SessionTarget, handing: Handing): void {
  pending.set(`${session.id}:${target}`, handing);
  void transport.locateSession(session.id, target);
}

/**
 * The service's answer to `locate`: opens the path with Windows' default app (or shows it in
 * the Explorer). Outside Tauri, or when it failed, an error toast says so.
 */
export async function openLocated(located: Located, tauri = isTauri()): Promise<void> {
  const key = `${located.id}:${located.target}`;
  const handing = pending.get(key);
  pending.delete(key);
  if (!handing) return;
  if (!located.windows_path) return showNotice("error", located.error ?? "No path");
  if (!tauri) return showNotice("error", `Only the Hive app opens ${located.windows_path}`);
  try {
    await openPath(located.windows_path, handing === "reveal");
  } catch (error) {
    showNotice("error", String(error));
  }
}

/** Copies `text`, saying so (or why not) in a toast. */
export const copy = (text: string, what: string) =>
  navigator.clipboard
    .writeText(text)
    .then(() => showNotice("info", `Copied the ${what}`))
    .catch((error) => showNotice("error", `Cannot copy the ${what}: ${error}`));

/** A session's name: its title, else its id. */
export const sessionName = (session: Session) => session.title ?? session.id;

/** Deletes the session's log once the user agrees. */
export function remove(session: Session): void {
  ask({
    title: "Delete session?",
    text: `Delete "${sessionName(session)}"? Its log is removed and it cannot be resumed.`,
    action: "Delete",
    run: () => void transport.deleteSession(session.id),
  });
}

/** A token count, short: "950", "84k", "1.2M". */
export function tokens(n: number): string {
  if (n < 1000) return `${n}`;
  // Below what would round to "1000k".
  if (n < 999_500) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** A session's tokens for its card ("84k ctx · 12k out"); null before any usage. */
export const sessionTokens = (x: Session) =>
  x.context_tokens > 0 ? `${tokens(x.context_tokens)} ctx · ${tokens(x.output_tokens)} out` : null;

/** "now", "5m ago", "3h ago", "2d ago", else the date. */
export function ago(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 60 * 24) return `${Math.floor(minutes / 60)}h ago`;
  if (minutes < 60 * 24 * 30) return `${Math.floor(minutes / (60 * 24))}d ago`;
  return new Date(ms).toLocaleDateString();
}
