import { isTauri } from "@tauri-apps/api/core";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  activateTab,
  type ServiceMessage,
  type Session,
  type SessionTarget,
  setNotice,
  useHive,
} from "./store";
import { openClaude } from "./terminals";
import { transport } from "./transport";

// What the sidebar's Sessions do with a Claude session. Claude runs in a Hive terminal as the
// user would run it; logs and folders open with Windows' own apps.

type Located = Extract<ServiceMessage, { type: "session_located" }>;
type Handing = "open" | "reveal";

/** How each located path goes to Windows, by `<id>:<target>`, until the service answers. */
const pending = new Map<string, Handing>();

/** Why a session running outside Hive is not resumed here. */
export const OUTSIDE = "This session runs in a terminal outside Hive: continue it there";

/** `claude`'s arguments to go on with a session, or to start a new one from it (`fork`). */
export const resumeArgs = (session: Session, fork = false) =>
  `--resume ${session.id}${fork ? " --fork-session" : ""}`;

/** A shell word for `text`, single-quoted (fish and POSIX shells read it the same). */
const quoted = (text: string) => `'${text.replaceAll("'", "'\\''")}'`;

/** What to type in any terminal to go on with the session. */
export const resumeCommand = (session: Session) =>
  `cd ${quoted(session.cwd)} && claude ${resumeArgs(session)}`;

/**
 * Goes on with the session: shows its terminal when it runs in one, else runs
 * `claude --resume` in a new terminal in its folder. `fork` always starts a new session.
 */
export async function resume(session: Session, fork = false): Promise<void> {
  const s = useHive.getState();
  const agent = s.agents[session.id];
  const tab = agent && s.tabs.find((t) => t.id === agent.terminal);
  if (tab && !fork) return activateTab(tab);
  // A second `claude` on the same session would write the same log.
  if (session.running && !fork) return setNotice(OUTSIDE);
  try {
    await openClaude(session.cwd, resumeArgs(session, fork));
  } catch (error) {
    setNotice(`Cannot open a terminal in ${session.cwd}: ${error}`);
  }
}

/**
 * The sessions that ran in Hive's terminals when the app last closed: each is resumed in a new
 * terminal in its folder, one after the other.
 */
export async function restore(sessions: { id: string; cwd: string }[]): Promise<void> {
  for (const { id, cwd } of sessions) {
    try {
      await openClaude(cwd, `--resume ${id}`);
    } catch (error) {
      setNotice(`Cannot resume the session in ${cwd}: ${error}`);
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
 * the Explorer). Outside Tauri, or when it failed, the status bar says so.
 */
export async function openLocated(located: Located, tauri = isTauri()): Promise<void> {
  const key = `${located.id}:${located.target}`;
  const handing = pending.get(key);
  pending.delete(key);
  if (!handing) return;
  if (!located.windows_path) return setNotice(located.error);
  if (!tauri) return setNotice(`Only the Hive app opens ${located.windows_path}`);
  try {
    await (handing === "reveal" ? revealItemInDir : openPath)(located.windows_path);
  } catch (error) {
    setNotice(String(error));
  }
}

/** Copies `text`, saying so (or why not) in the status bar. */
export const copy = (text: string, what: string) =>
  navigator.clipboard
    .writeText(text)
    .then(() => setNotice(`Copied the ${what}`))
    .catch((error) => setNotice(`Cannot copy the ${what}: ${error}`));

/** A session's name: its title, else its id. */
export const sessionName = (session: Session) => session.title ?? session.id;

/** Deletes the session's log once the user agrees. */
export function remove(session: Session): void {
  const sure = window.confirm(
    `Delete the session "${sessionName(session)}"? Its log is removed and it cannot be resumed.`,
  );
  if (sure) void transport.deleteSession(session.id);
}

/** "now", "5m ago", "3h ago", "2d ago", else the date. */
export function ago(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 60 * 24) return `${Math.floor(minutes / 60)}h ago`;
  if (minutes < 60 * 24 * 30) return `${Math.floor(minutes / (60 * 24))}d ago`;
  return new Date(ms).toLocaleDateString();
}
