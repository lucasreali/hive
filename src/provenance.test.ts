import { afterEach, expect, mock, spyOn, test } from "bun:test";
import type { LineCommit, LineEdit, ServiceMessage } from "./protocol";
import {
  closeProvenance,
  commitLine,
  editAuthor,
  editLine,
  NO_SESSION,
  probably,
  shortId,
  showSession,
  whoWrote,
} from "./provenance";
import { apply } from "./reduce";
import { initialState, type LineAsk, setDiffBase, useHive } from "./store";
import { transport } from "./transport";

afterEach(() => {
  mock.restore();
  useHive.setState(initialState, true);
});

const NOW = Date.UTC(2026, 10, 26, 12);
const HOUR = 3_600_000;
const edit: LineEdit = {
  session: "s",
  title: "Fix the login",
  deleted: false,
  project: "/r",
  worktree: "/r",
  agent: null,
  agent_type: null,
  tool: "Edit",
  tool_use_id: "toolu_1",
  ms: NOW - 3 * HOUR,
  user_modified: false,
  start: 120,
  len: 3,
};
const commit: LineCommit = {
  id: "abc1234def",
  author: "Ada",
  ms: NOW - 48 * HOUR,
  summary: "Keep sessions",
};
const ask: LineAsk = { worktree: "/r", path: "a.ts", line: 121, side: "worktree", x: 5, y: 6 };
const author = (origin: Extract<ServiceMessage, { type: "line_author" }>["origin"], line = 121) =>
  ({ type: "line_author", worktree: "/r", path: "a.ts", line, side: "worktree", origin }) as const;

test("the edit's wording: its session, tool, time, lines and what the user did", () => {
  expect(editAuthor(edit)).toBe("Fix the login");
  expect(editAuthor({ ...edit, title: null })).toBe("Untitled session");
  expect(editAuthor({ ...edit, agent: "a1", agent_type: "general-purpose" })).toBe(
    "general-purpose subagent of Fix the login",
  );
  expect(editAuthor({ ...edit, agent: "a1" })).toBe("Subagent of Fix the login");
  expect(editLine(edit, NOW)).toBe("Edited lines 120–122 · 3h ago");
  expect(editLine({ ...edit, len: 1, user_modified: true }, NOW)).toBe(
    "Edited line 120 · 3h ago · you changed it before accepting",
  );
  expect(editLine({ ...edit, deleted: true }, NOW)).toBe(
    "Edited lines 120–122 · 3h ago · session deleted",
  );
  expect(editLine({ ...edit, tool: "Write", len: 1 }, NOW)).toBe("Wrote line 120 · 3h ago");
  expect(editLine({ ...edit, tool: "MultiEdit" }, NOW)).toBe("Edited lines 120–122 · 3h ago");
  expect(shortId(commit.id)).toBe("abc1234");
  expect(probably(edit, commit, NOW)).toBe("Probably Fix the login, committed in abc1234 · 2d ago");
  expect(commitLine(commit, NOW)).toBe("abc1234 · Ada · 2d ago · Keep sessions");
  expect(NO_SESSION).toStartWith("No session recorded for this line.");
  // Never more than the record says.
  expect(NO_SESSION).not.toContain("verified");
});

test("asking sends the line with the worktree's diff base, and closes the menu", async () => {
  const sent = spyOn(transport, "whoWrote").mockResolvedValue();
  setDiffBase("/r", "branch");
  useHive.setState({ lineMenu: ask });
  whoWrote({ ...ask, side: "base" });
  expect(sent).toHaveBeenCalledWith("/r", "a.ts", 121, "base", "branch");
  expect(useHive.getState().lineMenu).toBeNull();
  expect(useHive.getState().provenance).toEqual({ ...ask, side: "base", origin: null });
});

test("a live session's answer opens the Sessions view at its edit; others show by the line", () => {
  spyOn(transport, "whoWrote").mockResolvedValue();
  // Nothing asked: an answer changes nothing.
  apply(author({ kind: "editor", ms: NOW }));
  expect(useHive.getState().provenance).toBeNull();

  whoWrote(ask);
  // A late answer for another line changes nothing.
  apply(author({ kind: "editor", ms: NOW }, 7));
  expect(useHive.getState().provenance?.origin).toBeNull();

  apply(author({ kind: "unknown", commit }));
  expect(useHive.getState().provenance).toEqual({ ...ask, origin: { kind: "unknown", commit } });
  closeProvenance();
  expect(useHive.getState().provenance).toBeNull();

  whoWrote(ask);
  useHive.setState({ openSessionFiles: ["other"] });
  apply(author({ kind: "session", edit, commit: null }));
  const s = useHive.getState();
  expect(s.provenance).toBeNull();
  expect(s.rightPanel).toBe("files");
  expect(s.panelView).toBe("sessions");
  expect(s.sessionFocus).toEqual({ edit, path: "a.ts" });
  expect(s.openSessionFiles).toEqual(["other", "s"]);

  // A probable one shows by the line; "Show session" does what a live one does.
  whoWrote(ask);
  apply(author({ kind: "session", edit, commit }));
  expect(useHive.getState().provenance?.origin).toEqual({ kind: "session", edit, commit });
  useHive.setState({ sessionFocus: null });
  showSession(edit, "a.ts");
  expect(useHive.getState().provenance).toBeNull();
  expect(useHive.getState().sessionFocus).toEqual({ edit, path: "a.ts" });
  // Its files list was open already.
  expect(useHive.getState().openSessionFiles).toEqual(["other", "s"]);
});

test("a session's files are kept by session", () => {
  const files = [
    { worktree: "/r", path: "a.ts", edits: 2, lines_now: 3, first_line: 4, last_ms: NOW },
  ];
  apply({ type: "session_files", id: "s", files, truncated: true });
  expect(useHive.getState().sessionFiles).toEqual({ s: { files, truncated: true } });
});
