import type { LineCommit, LineEdit, ServiceMessage } from "./protocol";
import { ago } from "./sessions";
import {
  diffBase,
  type HiveState,
  type LineAsk,
  type Provenance,
  showFailure,
  useHive,
} from "./store";
import { transport } from "./transport";

// "Who wrote this line?" (27.5, spike §10): the service answers from what Hive recorded; the UI
// only shows it, by the line or as the session's row. What Hive recorded is advisory, never
// proof: an answer found through a commit says "probably".

type LineAuthor = Extract<ServiceMessage, { type: "line_author" }>;

/** What the popover says when no session is recorded for a line. */
export const NO_SESSION =
  "No session recorded for this line. It was changed outside an agent's Edit or Write (a shell command, another editor, git), or before Hive recorded the file.";

/** A commit's short id, as git shows it. */
export const shortId = (id: string) => id.slice(0, 7);

/** The session that made `edit`: its title, or "<type> subagent of <title>" for a subagent's. */
export function editAuthor(edit: LineEdit): string {
  const title = edit.title ?? "Untitled session";
  if (edit.agent_type) return `${edit.agent_type} subagent of ${title}`;
  return edit.agent ? `Subagent of ${title}` : title;
}

/** The verb a session's row tells an edit by: "Wrote" for a Write, "Edited" for the rest. */
const VERB: Record<string, string> = { Write: "Wrote" };

/** The edit as its session's row tells it (28.22), e.g. "Edited lines 120–122 · 3h ago". */
export function editLine(edit: LineEdit, now = Date.now()): string {
  const last = edit.start + edit.len - 1;
  const lines = edit.len > 1 ? `lines ${edit.start}–${last}` : `line ${edit.start}`;
  const changed = edit.user_modified && "you changed it before accepting";
  const deleted = edit.deleted && "session deleted";
  const verb = VERB[edit.tool] ?? "Edited";
  return [`${verb} ${lines}`, ago(edit.ms, now), changed, deleted].filter(Boolean).join(" · ");
}

/** "Probably <session>, committed in <id> · <time>": found through the commit, not followed. */
export const probably = (edit: LineEdit, commit: LineCommit, now = Date.now()) =>
  `Probably ${editAuthor(edit)}, committed in ${shortId(commit.id)} · ${ago(commit.ms, now)}`;

/** A commit's line under the unknown answer: its short id, author, time and summary. */
export const commitLine = (commit: LineCommit, now = Date.now()) =>
  [shortId(commit.id), commit.author, ago(commit.ms, now), commit.summary].join(" · ");

/** Asks the service who wrote `ask`'s line, at the diff's base for its old side. */
export function whoWrote(ask: LineAsk): void {
  const s = useHive.getState();
  useHive.setState({ lineMenu: null, provenance: { ...ask, origin: null } });
  const { worktree, path, line, side } = ask;
  void showFailure(transport.whoWrote(worktree, path, line, side, diffBase(s, worktree)));
}

/**
 * The Sessions view at `edit` (spike §10, Q4): the right panel's Sessions tab, that session's
 * row with its "Files changed" open and `path` highlighted. The worktree shown stays.
 */
export function showEdit(s: HiveState, edit: LineEdit, path: string): Partial<HiveState> {
  const open = s.openSessionFiles;
  return {
    provenance: null,
    rightPanel: "files",
    panelView: "sessions",
    sessionFocus: { edit, path },
    openSessionFiles: open.includes(edit.session) ? open : [...open, edit.session],
  };
}

/** "Show session" of a probable answer: as a live one. */
export const showSession = (edit: LineEdit, path: string) =>
  useHive.setState((s) => showEdit(s, edit, path));

/** Closes the answer's popover. */
export const closeProvenance = () => useHive.setState({ provenance: null });

/** Whether `m` answers the line `asked` is about. */
const answers = (asked: Provenance, m: LineAuthor) =>
  asked.worktree === m.worktree &&
  asked.path === m.path &&
  asked.line === m.line &&
  asked.side === m.side;

/**
 * The service's answer for the line last asked: a session's edit followed live shows as its row
 * in the Sessions view; any other answer in the popover by the line. A late answer for another
 * line changes nothing.
 */
export function authorAnswered(s: HiveState, m: LineAuthor): Partial<HiveState> {
  const asked = s.provenance;
  if (!asked || !answers(asked, m)) return {};
  const { origin } = m;
  if (origin.kind === "session" && origin.commit === null) return showEdit(s, origin.edit, m.path);
  return { provenance: { ...asked, origin } };
}
