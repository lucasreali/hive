import { afterEach, expect, jest, spyOn, test } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { notice, noticeKind } from "../test/notice";
import { connect } from "./connect";
import type { ServiceMessage } from "./protocol";
import { apply } from "./reduce";
import {
  ago,
  copy,
  locate,
  MOVE_WAIT_MS,
  onMoved,
  openLocated,
  remove,
  restore,
  resume,
  resumeArgs,
  sessionName,
  sessionTokens,
  tokens,
} from "./sessions";
import { initialState, useHive } from "./store";
import { closeTerminal } from "./terminals";
import { transport } from "./transport";
import { MOCK_SESSIONS } from "./transport/mock";

afterEach(() => {
  clearMocks();
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  useHive.setState(initialState, true);
});

const [session, stopped] = MOCK_SESSIONS;

test("resume arguments go on with the session, or fork it", () => {
  expect(resumeArgs(session)).toBe(`--resume ${session.id}`);
  expect(resumeArgs(session, true)).toBe(`--resume ${session.id} --fork-session`);
  expect(sessionName(session)).toBe("Fix the login redirect");
  expect(sessionName({ ...session, title: null })).toBe(session.id);
});

test("resume shows a running session's terminal, else runs claude --resume in its folder", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(3);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  await resume({ ...stopped, config_dir: "/w" });
  expect(open.mock.calls[0]?.[0]).toBe(stopped.cwd);
  // As the account it ran as.
  expect(open.mock.calls[0]?.[4]).toEqual({ config_dir: "/w" });
  expect(write).toHaveBeenCalledWith(3, `claude --resume ${stopped.id}\r`);

  // Running in terminal 3: its tab is shown instead; a fork still opens a new terminal.
  const { cwd } = stopped;
  apply({ type: "agent_detected", channel: 3, id: stopped.id, project: null, worktree: null, cwd });
  useHive.setState({ activeTab: null });
  await resume(stopped);
  expect(open).toHaveBeenCalledTimes(1);
  expect(useHive.getState().activeTab).toBe(3);
  await resume(stopped, true);
  expect(write).toHaveBeenLastCalledWith(3, `claude --resume ${stopped.id} --fork-session\r`);

  // Running outside Hive: not resumed a second time, with nothing said; a fork is fine.
  await resume(session);
  expect(notice()).toBeNull();
  expect(open).toHaveBeenCalledTimes(2);
  await resume(session, true);
  expect(open).toHaveBeenCalledTimes(3);

  open.mockRejectedValue("no such folder");
  await resume({ ...stopped, id: "other" });
  expect(notice()).toBe(`Cannot open a terminal in ${stopped.cwd}: no such folder`);
  open.mockRestore();
  write.mockRestore();
});

test("shared sessions resume as the current account, their log moved there first (21.10)", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(5);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  // The service answers each move, through the app's message handler: done, or why not.
  let onMessage: (message: ServiceMessage) => void = () => {};
  const connected = spyOn(transport, "connect").mockImplementation(async (handler) => {
    onMessage = handler;
  });
  await connect();
  connected.mockRestore();
  let answer: string | null = null;
  const moved = spyOn(transport, "moveSession").mockImplementation(async (id, config_dir) => {
    queueMicrotask(() =>
      onMessage(
        answer === null
          ? { type: "session_moved", id, config_dir }
          : { type: "move_session_failed", id, message: answer },
      ),
    );
  });
  const claude = (share_sessions: boolean, account: string | null) => ({
    accounts: [{ name: "Work", config_dir: "/w" }],
    account,
    default_name: "Default",
    share_sessions,
  });
  const set = (share: boolean, account: string | null) =>
    useHive.setState({ settings: { ...initialState.settings, claude: claude(share, account) } });
  const asked = () => open.mock.calls.at(-1)?.[4];

  // Off: as the account it ran as, nothing moved.
  set(false, "/w");
  await resume(stopped);
  expect(asked()).toEqual({ config_dir: null });
  expect(moved).not.toHaveBeenCalled();
  // On: as the current account, its log moved there first.
  set(true, "/w");
  await resume(stopped);
  expect(moved).toHaveBeenLastCalledWith(stopped.id, "/w");
  expect(asked()).toEqual({ config_dir: "/w" });
  // A current account no account has is the default one: already its own, nothing moved.
  set(true, "/gone");
  await resume(stopped);
  expect(moved).toHaveBeenCalledTimes(1);
  expect(asked()).toEqual({ config_dir: null });
  // "Continue in" names the account, whatever the setting.
  set(false, null);
  await resume(stopped, false, "/w");
  expect(moved).toHaveBeenLastCalledWith(stopped.id, "/w");
  expect(asked()).toEqual({ config_dir: "/w" });
  // A fork of a session running elsewhere stays in its own account: its log cannot move.
  set(true, "/w");
  await resume(session, true);
  expect(moved).toHaveBeenCalledTimes(2);
  expect(asked()).toEqual({ config_dir: session.config_dir });
  // A refused move opens nothing and says why.
  answer = "the account already has a log of session x";
  const opened = open.mock.calls.length;
  await resume(stopped);
  expect(open.mock.calls.length).toBe(opened);
  expect(notice()).toBe("Cannot move the session: the account already has a log of session x");
  // An answer nobody waits for is ignored.
  onMoved({ type: "session_moved", id: "nobody", config_dir: null });
  open.mockRestore();
  write.mockRestore();
  moved.mockRestore();
});

test("a move waits for its answer at most 30 s, and one move of a session at a time", async () => {
  jest.useFakeTimers();
  const open = spyOn(transport, "openTerminal").mockResolvedValue(5);
  // The service never answers.
  const moved = spyOn(transport, "moveSession").mockResolvedValue();
  const claude = {
    accounts: [{ name: "Work", config_dir: "/w" }],
    account: "/w",
    default_name: "Default",
  };
  const settings = { ...initialState.settings, claude: { ...claude, share_sessions: true } };
  useHive.setState({ settings });
  const first = resume(stopped);
  await resume(stopped);
  expect(notice()).toBe("Cannot move the session: it is already being moved");
  expect(moved).toHaveBeenCalledTimes(1);
  jest.advanceTimersByTime(MOVE_WAIT_MS - 1);
  await Promise.resolve();
  expect(notice()).toBe("Cannot move the session: it is already being moved");
  jest.advanceTimersByTime(1);
  await first;
  expect(notice()).toBe("Cannot move the session: the service did not answer");
  expect(open).not.toHaveBeenCalled();
  // A late answer finds nobody waiting; the next move is asked again.
  onMoved({ type: "session_moved", id: stopped.id, config_dir: "/w" });
  void resume(stopped);
  expect(moved).toHaveBeenCalledTimes(2);
  jest.advanceTimersByTime(MOVE_WAIT_MS);
  jest.useRealTimers();
  open.mockRestore();
  moved.mockRestore();
});

test("the sessions open when the app last closed are resumed, one after the other", async () => {
  const open = spyOn(transport, "openTerminal")
    .mockResolvedValueOnce(4)
    .mockRejectedValueOnce("gone");
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  await restore([
    { id: "a", cwd: "/r", config_dir: "/w" },
    { id: "b", cwd: "/r/x", config_dir: null },
  ]);
  expect(open.mock.calls.map((c) => [c[0], c[4]])).toEqual([
    ["/r", { config_dir: "/w" }],
    ["/r/x", { config_dir: null }],
  ]);
  expect(write).toHaveBeenCalledWith(4, "claude --resume a\r");
  expect(notice()).toBe("Cannot resume the session in /r/x: gone");
  open.mockRestore();
  write.mockRestore();
});

test("a located log or folder opens, or is revealed, with Windows' apps", async () => {
  const asked = spyOn(transport, "locateSession").mockResolvedValue();
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
  });
  const located = (target: "log" | "folder", windows_path: string | null, error = null) => ({
    type: "session_located" as const,
    id: session.id,
    target,
    windows_path,
    error,
  });
  // Nothing asked: nothing happens.
  await openLocated(located("log", "C:\\x"), true);
  expect(calls).toEqual([]);

  locate(session, "log", "open");
  expect(asked).toHaveBeenCalledWith(session.id, "log");
  await openLocated(located("log", "C:\\log"), true);
  locate(session, "log", "reveal");
  await openLocated(located("log", "C:\\log"), true);
  locate(session, "folder", "open");
  await openLocated(located("folder", "C:\\dir"), true);
  expect(calls).toEqual([
    ["open_path", { path: "C:\\log", reveal: false }],
    ["open_path", { path: "C:\\log", reveal: true }],
    ["open_path", { path: "C:\\dir", reveal: false }],
  ]);

  locate(session, "log", "open");
  await openLocated({ ...located("log", null), error: "wslpath failed: x" }, true);
  expect([notice(), noticeKind()]).toEqual(["wslpath failed: x", "error"]);
  locate(session, "log", "open");
  await openLocated({ ...located("log", null), error: null }, true);
  expect(notice()).toBe("No path");
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"), false);
  expect(notice()).toBe("Only the Hive app opens C:\\log");
  mockIPC(() => {
    throw new Error("no app");
  });
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"), true);
  expect(notice()).toContain("no app");
  // A log no app opens, even the text editor (D21.1): the app's words, never an exit status.
  mockIPC(() => {
    throw "No app opens 570714e7.jsonl";
  });
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"), true);
  expect([notice(), noticeKind()]).toEqual(["No app opens 570714e7.jsonl", "error"]);
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"));
  asked.mockRestore();
});

test("copying says so in a fading toast, or why not in one that stays", async () => {
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  await copy("abc", "session id");
  expect(writeText).toHaveBeenCalledWith("abc");
  expect([notice(), noticeKind()]).toEqual(["Copied the session id", "info"]);
  writeText.mockRejectedValue("denied");
  await copy("abc", "log path");
  expect([notice(), noticeKind()]).toEqual(["Cannot copy the log path: denied", "error"]);
  writeText.mockRestore();
});

test("deleting asks first in a Hive dialog, never the WebView's", () => {
  const deleted = spyOn(transport, "deleteSession").mockResolvedValue();
  const native = spyOn(window, "confirm");
  remove(session);
  const { modal, question } = useHive.getState();
  expect(modal).toBe("confirm");
  expect([question?.title, question?.text, question?.action]).toEqual([
    "Delete session?",
    'Delete "Fix the login redirect"? Its log is removed and it cannot be resumed.',
    "Delete",
  ]);
  expect(deleted).not.toHaveBeenCalled();
  question?.run();
  expect(deleted).toHaveBeenCalledWith(session.id);
  expect(native).not.toHaveBeenCalled();
  deleted.mockRestore();
  native.mockRestore();
});

test("token counts are short, and a session without usage shows none", () => {
  expect([999, 1000, 84_400, 999_499, 999_500, 1_250_000].map(tokens)).toEqual([
    "999",
    "1k",
    "84k",
    "999k",
    "1.0M",
    "1.3M",
  ]);
  const used = { ...session, context_tokens: 84_000, output_tokens: 12_300 };
  expect(sessionTokens(used)).toBe("84k ctx · 12k out");
  expect(sessionTokens({ ...session, context_tokens: 0 })).toBeNull();
});

test("how long ago, in the largest unit", () => {
  const now = 1_000_000_000_000;
  const minutes = (n: number) => now - n * 60_000;
  expect(ago(now, now)).toBe("now");
  expect(ago(minutes(5), now)).toBe("5m ago");
  expect(ago(minutes(59), now)).toBe("59m ago");
  expect(ago(minutes(60), now)).toBe("1h ago");
  expect(ago(minutes(60 * 24), now)).toBe("1d ago");
  expect(ago(minutes(60 * 24 * 29), now)).toBe("29d ago");
  expect(ago(minutes(60 * 24 * 30), now)).toBe(
    new Date(minutes(60 * 24 * 30)).toLocaleDateString(),
  );
  expect(ago(Date.now())).toBe("now");
});
