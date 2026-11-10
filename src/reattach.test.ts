import { afterAll, expect, spyOn, test } from "bun:test";
import type { Terminal } from "@xterm/xterm";
import { notice } from "../test/notice";
import { connect } from "./connect";
import type { ServiceMessage } from "./protocol";
import { initialState, useHive } from "./store";
import { closeTerminal, openTerminal, terminal } from "./terminals";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 18.21: after a WebContent crash the app keeps the terminals and tells the reloaded page about
// each one (`terminal_kept`), which reattaches to it.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const screen = (term: Terminal) =>
  new Promise<string>((resolve) =>
    term.write("", () => resolve(term.buffer.active.getLine(0)?.translateToString(true) ?? "")),
  );

afterAll(() => useHive.setState(initialState, true));

test("a kept terminal gets its tab and its output back in the reloaded page", async () => {
  await connect();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  // Opened by the page that crashed: its output went there.
  const kept = await transport.openTerminal(shop.path, 80, 24, () => {});
  await settle();
  useHive.setState(initialState, true);
  let onMessage: (message: ServiceMessage) => void = () => {};
  const connected = spyOn(transport, "connect").mockImplementation(async (handler) => {
    onMessage = handler;
  });
  await connect();
  connected.mockRestore();
  const resized = spyOn(transport, "resizeTerminal");

  onMessage({
    type: "terminal_kept",
    channel: kept,
    cwd: shop.path,
    worktree: shop.path,
    account: "Work",
  });
  await settle();
  const s = useHive.getState();
  expect(s.tabs).toEqual([{ id: kept, cwd: shop.path }]);
  expect(s.activeTab).toBe(kept);
  expect(s.terminals[kept]).toMatchObject({ exited: false, worktree: shop.path, account: "Work" });
  // Resized from 80×24, so a full-screen program draws again once the pane is fitted.
  expect(resized).toHaveBeenCalledWith(kept, 80, 24);
  resized.mockRestore();
  // Its output reaches the new Terminal (the mock echoes what is typed).
  await transport.writeTerminal(kept, "x");
  await settle();
  expect(await screen(terminal(kept) as Terminal)).toContain("x");

  // One the app no longer has is shown as a failure.
  onMessage({ type: "terminal_kept", channel: 999, cwd: shop.path, worktree: null, account: null });
  await settle();
  expect(notice()).toContain("Cannot open a terminal: Error: no such terminal");
  expect(terminal(999)).toBeUndefined();
  closeTerminal(kept);
});

// 26.16: a remote service's terminals after a dropped link or an app restart. The Tauri side
// answers the service's `terminals_kept`: `terminal_kept` for one the page does not have,
// `terminal_reattached` for one it shows; a reset reaches the screen as RIS (`ESC c`).

/** Connects with a captured handler; returns it. */
async function connected(): Promise<(message: ServiceMessage) => void> {
  let onMessage: (message: ServiceMessage) => void = () => {};
  const spy = spyOn(transport, "connect").mockImplementation(async (handler) => {
    onMessage = handler;
  });
  await connect();
  spy.mockRestore();
  return onMessage;
}

/** Every line of the terminal's buffer, scrollback included, without trailing blanks. */
const lines = (term: Terminal) =>
  new Promise<string[]>((resolve) =>
    term.write("", () => {
      const buffer = term.buffer.active;
      const all = Array.from({ length: buffer.length }, (_, i) =>
        (buffer.getLine(i)?.translateToString(true) ?? "").trimEnd(),
      );
      resolve(all.filter((line) => line !== ""));
    }),
  );

test("a terminal the app restarted without opens as a tab under its worktree, from the ring", async () => {
  useHive.setState(initialState, true);
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  // Its channel in the mock; its own `terminal_opened` goes to the page that is gone.
  const id = await transport.openTerminal(shop.path, 80, 24, () => {});
  await settle();
  const onMessage = await connected();
  let output: (bytes: Uint8Array) => void = () => {};
  const attached = spyOn(transport, "attachTerminal").mockImplementation(async (_, onData) => {
    output = onData;
  });
  const resized = spyOn(transport, "resizeTerminal");
  onMessage({
    type: "terminal_kept",
    channel: id,
    cwd: shop.path,
    worktree: shop.path,
    account: "Work",
  });
  await settle();
  expect(attached).toHaveBeenCalledTimes(1);
  attached.mockRestore();
  const s = useHive.getState();
  expect(s.tabs).toEqual([{ id, cwd: shop.path }]);
  expect(s.terminals[id]).toMatchObject({ worktree: shop.path, account: "Work" });
  // The redraw: one column wider, then back, in that order.
  expect(resized.mock.calls.slice(0, 2)).toEqual([
    [id, 81, 24],
    [id, 80, 24],
  ]);
  resized.mockRestore();

  // The ring replayed; then a reset: the screen is cleared, scrollback too, and the whole
  // ring written again.
  const encode = (text: string) => new TextEncoder().encode(text);
  const term = terminal(id) as Terminal;
  output(encode("old 1\r\nold 2\r\n"));
  expect(await lines(term)).toEqual(["old 1", "old 2"]);
  output(encode("\x1bc"));
  output(encode("ring\r\n"));
  expect(await lines(term)).toEqual(["ring"]);
  closeTerminal(id);
});

test("a terminal the page shows, attached again, is redrawn one column wider and back", async () => {
  useHive.setState(initialState, true);
  await connect();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const id = await openTerminal(shop.path);
  const term = terminal(id) as Terminal;
  const onMessage = await connected();
  const resized = spyOn(transport, "resizeTerminal");
  onMessage({ type: "terminal_reattached", channel: id });
  expect(resized.mock.calls).toEqual([
    [id, term.cols + 1, term.rows],
    [id, term.cols, term.rows],
  ]);
  // One the page does not show: nothing to redraw.
  resized.mockClear();
  onMessage({ type: "terminal_reattached", channel: 999 });
  expect(resized).not.toHaveBeenCalled();
  resized.mockRestore();
  closeTerminal(id);
});
