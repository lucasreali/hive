import { afterEach, expect, spyOn, test } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import {
  ago,
  copy,
  locate,
  openAsChat,
  openLocated,
  remove,
  restore,
  resume,
  resumeArgs,
  resumeCommand,
  sessionName,
  sessionTokens,
  tokens,
} from "./sessions";
import { apply, initialState, useHive } from "./store";
import { closeTerminal } from "./terminals";
import { transport } from "./transport";
import { MOCK_SESSIONS } from "./transport/mock";

afterEach(() => {
  clearMocks();
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  useHive.setState(initialState, true);
});

const [session, stopped] = MOCK_SESSIONS;
const notice = () => useHive.getState().notice;

test("resume commands go on with the session, or fork it", () => {
  expect(resumeArgs(session)).toBe(`--resume ${session.id}`);
  expect(resumeArgs(session, true)).toBe(`--resume ${session.id} --fork-session`);
  const odd = { ...session, cwd: "/home/me/it's here" };
  expect(resumeCommand(odd)).toBe(`cd '/home/me/it'\\''s here' && claude --resume ${session.id}`);
  expect(sessionName(session)).toBe("Fix the login redirect");
  expect(sessionName({ ...session, title: null })).toBe(session.id);
});

test("resume shows a running session's terminal, else runs claude --resume in its folder", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(3);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  await resume(stopped);
  expect(open.mock.calls[0]?.[0]).toBe(stopped.cwd);
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

test("the sessions open when the app last closed are resumed, one after the other", async () => {
  const open = spyOn(transport, "openTerminal")
    .mockResolvedValueOnce(4)
    .mockRejectedValueOnce("gone");
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const chat = spyOn(transport, "openChat").mockResolvedValue(6);
  await restore([
    { id: "a", cwd: "/r", kind: "terminal" },
    { id: "b", cwd: "/r/x", kind: "terminal" },
    { id: "c", cwd: "/r/y", kind: "chat" },
  ]);
  expect(open.mock.calls.map((c) => c[0])).toEqual(["/r", "/r/x"]);
  expect(write).toHaveBeenCalledWith(4, "claude --resume a\r");
  expect(notice()).toBe("Cannot resume the session in /r/x: gone");
  // A chat comes back as a chat.
  expect(chat.mock.calls).toEqual([["/r/y", "c", null]]);
  expect(useHive.getState().tabs.at(-1)).toEqual({ id: 6, cwd: "/r/y", kind: "chat" });
  open.mockRestore();
  write.mockRestore();
  chat.mockRestore();
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
    ["plugin:opener|open_path", { path: "C:\\log" }],
    ["plugin:opener|reveal_item_in_dir", { paths: ["C:\\log"] }],
    ["plugin:opener|open_path", { path: "C:\\dir" }],
  ]);

  locate(session, "log", "open");
  await openLocated({ ...located("log", null), error: "wslpath failed: x" }, true);
  expect(notice()).toBe("wslpath failed: x");
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"), false);
  expect(notice()).toBe("Only the Hive app opens C:\\log");
  mockIPC(() => {
    throw new Error("no app");
  });
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"), true);
  expect(notice()).toContain("no app");
  locate(session, "log", "open");
  await openLocated(located("log", "C:\\log"));
  asked.mockRestore();
});

test("copying says so in the status bar, or why not", async () => {
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  await copy("abc", "session id");
  expect(writeText).toHaveBeenCalledWith("abc");
  expect(notice()).toBe("Copied the session id");
  writeText.mockRejectedValue("denied");
  await copy("abc", "log path");
  expect(notice()).toBe("Cannot copy the log path: denied");
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

test("a session opens as a chat in its folder, or says why not", async () => {
  const chat = spyOn(transport, "openChat").mockResolvedValueOnce(7).mockRejectedValueOnce("no");
  await openAsChat(stopped);
  expect(chat.mock.calls).toEqual([[stopped.cwd, stopped.id, null]]);
  expect(useHive.getState().activeTab).toBe(7);
  await openAsChat(stopped);
  expect(notice()).toBe(`Cannot open a chat in ${stopped.cwd}: no`);
  chat.mockRestore();
});
