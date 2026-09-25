import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { asMac } from "../test/mac";
import { apply, DEFAULT_SETTINGS, initialState, setWidth, shownTerminals, useHive } from "./store";
import { transport } from "./transport";

// happy-dom has no WebGL: a fake addon records what the manager does with the renderer.
let webglFails = false;
const addons: FakeWebgl[] = [];
class FakeWebgl {
  disposed = false;
  lose = () => {};
  onContextLoss(listener: () => void) {
    this.lose = listener;
    return { dispose() {} };
  }
  activate() {
    if (webglFails) throw new Error("WebGL is not available");
    addons.push(this);
  }
  dispose() {
    this.disposed = true;
  }
}
mock.module("@xterm/addon-webgl", () => ({ WebglAddon: FakeWebgl }));

// Only the manager's ResizeObserver exists in these tests; it is triggered by hand.
let observed: { fire: () => void; disconnected: boolean } | null = null;
globalThis.ResizeObserver = class {
  constructor(callback: ResizeObserverCallback) {
    observed = { fire: () => callback([], this), disconnected: false };
  }
  observe() {}
  unobserve() {}
  disconnect() {
    if (observed) observed.disconnected = true;
  }
};

const {
  closeTerminal,
  interceptKeys,
  mountTerminals,
  openTerminal,
  showTerminal,
  showTerminals,
  splitTerminal,
  terminal,
} = await import("./terminals");

const written = (term: Terminal) =>
  new Promise<string>((resolve) =>
    term.write("", () => resolve(term.buffer.active.getLine(0)?.translateToString(true) ?? "")),
  );
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

let host: HTMLDivElement;
let unmount: () => void;
const opened: number[] = [];

beforeEach(async () => {
  useHive.setState(initialState, true);
  await transport.connect(apply);
  host = document.createElement("div");
  document.body.append(host);
  unmount = mountTerminals(host);
  webglFails = false;
  addons.length = 0;
});

afterEach(async () => {
  for (const id of opened.splice(0)) closeTerminal(id);
  unmount();
  host.remove();
  interceptKeys(() => false);
  // The mock's answers to `connect` (welcome, projects) arrive on later ticks: let them land
  // now, then reset, so none leaks into the next test file.
  await new Promise((resolve) => setTimeout(resolve, 0));
  useHive.setState(initialState, true);
});

async function open(cwd = "/w") {
  const id = await openTerminal(cwd);
  opened.push(id);
  return { id, term: terminal(id) as Terminal };
}

test("output written before the tab is shown is kept, and the tab opens shown", async () => {
  const openSpy = spyOn(transport, "openTerminal").mockImplementation(async (_c, _w, _h, out) => {
    out(new TextEncoder().encode("early"));
    return 99;
  });
  const { id, term } = await open("/repo/wt");
  expect(id).toBe(99);
  expect(openSpy.mock.calls[0].slice(0, 3)).toEqual(["/repo/wt", 80, 24]);
  openSpy.mockRestore();
  expect(await written(term)).toBe("early");
  expect(term.options.scrollback).toBe(5000);
  const s = useHive.getState();
  expect([s.tabs, s.activeTab, s.selection]).toEqual([
    [{ id: 99, cwd: "/repo/wt" }],
    99,
    "/repo/wt",
  ]);
});

test("the scrollback setting applies to new terminals", async () => {
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.terminal.scrollback = 42;
  useHive.setState({ settings });
  const { term } = await open();
  expect(term.options.scrollback).toBe(42);
});

test("new settings apply at once to open terminals, and the shown one refits", async () => {
  const { id, term } = await open();
  showTerminal(id);
  const hidden = (await open()).term;
  const fit = spyOn(FitAddon.prototype, "fit");
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.terminal = {
    font_family: "Fira Code",
    font_size: 20,
    scrollback: 1234,
    cursor_style: "bar",
    cursor_blink: true,
    copy_on_select: false,
  };
  settings.appearance.theme = "one-light";
  apply({ type: "settings", settings });
  for (const t of [term, hidden]) {
    expect(t.options).toMatchObject({
      fontFamily: "Fira Code",
      fontSize: 20,
      scrollback: 1234,
      cursorStyle: "bar",
      cursorBlink: true,
    });
    expect(t.options.theme?.background).toBe("#fafafa");
  }
  expect(fit).toHaveBeenCalledTimes(1);
  // Anything else changing in the store leaves them alone.
  useHive.setState({ notice: "x" });
  expect(fit).toHaveBeenCalledTimes(1);
  settings.appearance.theme = "one-dark";
  apply({ type: "settings", settings: structuredClone(settings) });
  expect(term.options.theme?.background).toBe("#282c33");
  fit.mockRestore();
  // No terminal shown: nothing to refit.
  showTerminal(null);
  apply({ type: "settings", settings: DEFAULT_SETTINGS });
  expect(hidden.options.fontSize).toBe(13);
  // Unmounted, the settings no longer reach them.
  unmount();
  apply({ type: "settings", settings });
  expect(hidden.options.fontSize).toBe(13);
  unmount = mountTerminals(host);
});

test("copy on select copies a new selection, only when set", async () => {
  const copy = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const { id, term } = await open();
  showTerminal(id);
  term.write("hello");
  await written(term);
  term.selectAll();
  expect(copy).not.toHaveBeenCalled();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.terminal.copy_on_select = true;
  apply({ type: "settings", settings });
  term.clearSelection();
  expect(copy).not.toHaveBeenCalled();
  term.selectAll();
  expect(copy).toHaveBeenCalledWith(term.getSelection());
  copy.mockRestore();
});

test("a refused open leaves no terminal and no tab", async () => {
  const openSpy = spyOn(transport, "openTerminal").mockRejectedValue(new Error("disconnected"));
  await expect(openTerminal("/w")).rejects.toThrow("disconnected");
  openSpy.mockRestore();
  expect(useHive.getState().tabs).toEqual([]);
});

test("input goes to the service until the shell exits; size changes resize the PTY", async () => {
  const write = spyOn(transport, "writeTerminal");
  const resize = spyOn(transport, "resizeTerminal");
  const { id, term } = await open();
  term.input("ls\r");
  expect(write).toHaveBeenLastCalledWith(id, "ls\r");
  term.resize(100, 30);
  expect(resize).toHaveBeenLastCalledWith(id, 100, 30);
  apply({ type: "terminal_exited", channel: id, code: 0 });
  term.input("more");
  expect(write).toHaveBeenCalledTimes(1);
  write.mockRestore();
  resize.mockRestore();
});

test("only the shown terminal is rendered, with WebGL; hiding it frees the renderer", async () => {
  const one = await open();
  const two = await open();
  const pane = (t: Terminal) => t.element?.parentElement as HTMLElement;
  showTerminal(one.id);
  expect(pane(one.term).parentElement).toBe(host);
  expect(pane(one.term).hidden).toBe(false);
  // Never shown: not even opened, yet it keeps its output.
  expect(two.term.element).toBeUndefined();
  const [first] = addons;
  expect(first.disposed).toBe(false);

  showTerminal(two.id);
  expect(first.disposed).toBe(true);
  expect(pane(one.term).hidden).toBe(true);
  expect(pane(two.term).hidden).toBe(false);
  expect(addons).toHaveLength(2);

  showTerminal(null);
  expect(addons[1].disposed).toBe(true);
  expect(pane(two.term).hidden).toBe(true);
});

test("a lost WebGL context falls back to the DOM renderer; no WebGL at all is fine too", async () => {
  const { id } = await open();
  showTerminal(id);
  const [addon] = addons;
  addon.lose();
  expect(addon.disposed).toBe(true);
  // Shown again later, it tries WebGL again.
  showTerminal(id);
  expect(addons).toHaveLength(2);
  addon.lose(); // a stale loss changes nothing
  expect(addons[1].disposed).toBe(false);

  webglFails = true;
  const other = await open();
  showTerminal(other.id);
  expect(other.term.element).toBeDefined();
  expect(addons).toHaveLength(2);
});

test("a terminal shown before the host mounts appears once it does", async () => {
  const { id, term } = await open();
  unmount();
  showTerminal(id);
  expect(term.element).toBeUndefined();
  unmount = mountTerminals(host);
  expect(term.element?.parentElement?.parentElement).toBe(host);
});

test("resizes of the host are debounced into one fit of the shown terminal", async () => {
  const fit = spyOn(FitAddon.prototype, "fit");
  const { id } = await open();
  showTerminal(id);
  fit.mockClear();
  observed?.fire();
  observed?.fire();
  await new Promise((resolve) => setTimeout(resolve, 80));
  expect(fit).toHaveBeenCalledTimes(1);
  // After unmount a pending fit is dropped and the observer is gone.
  observed?.fire();
  unmount();
  expect(observed?.disconnected).toBe(true);
  await new Promise((resolve) => setTimeout(resolve, 80));
  expect(fit).toHaveBeenCalledTimes(1);
  unmount = mountTerminals(host);
  fit.mockRestore();
});

function press(term: Terminal, key: string, init: KeyboardEventInit = {}, type = "keydown") {
  const event = new KeyboardEvent(type, {
    key,
    ctrlKey: true,
    shiftKey: true,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  term.textarea?.dispatchEvent(event);
  return event;
}

test("Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes, nothing reaches the shell", async () => {
  const write = spyOn(transport, "writeTerminal");
  const copy = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const read = spyOn(navigator.clipboard, "readText").mockResolvedValue("pasted");
  const { id, term } = await open();
  showTerminal(id);
  await new Promise<void>((resolve) => term.write("hello world", resolve));

  press(term, "C"); // nothing selected: nothing copied
  expect(copy).not.toHaveBeenCalled();
  term.select(0, 0, 5);
  expect(press(term, "C").defaultPrevented).toBe(true);
  expect(copy).toHaveBeenCalledWith("hello");
  press(term, "C", {}, "keyup");
  expect(copy).toHaveBeenCalledTimes(1);

  expect(press(term, "V").defaultPrevented).toBe(true);
  await settle();
  expect(read).toHaveBeenCalledTimes(1);
  expect(write.mock.calls).toEqual([[id, "pasted"]]);
  copy.mockRestore();
  read.mockRestore();
  write.mockRestore();
});

test("other keys go to the shell unless an app shortcut takes them first", async () => {
  const write = spyOn(transport, "writeTerminal");
  const { id, term } = await open();
  showTerminal(id);
  press(term, "c", { shiftKey: false, keyCode: 67 }); // Ctrl+C
  expect(write).toHaveBeenLastCalledWith(id, "\x03");
  press(term, "C", { altKey: true, keyCode: 67 }); // not the copy chord
  expect(write).toHaveBeenCalledTimes(2);

  const seen: string[] = [];
  interceptKeys((event) => {
    seen.push(event.key);
    return event.key === "T";
  });
  press(term, "T", { keyCode: 84 });
  expect(seen).toEqual(["T"]);
  expect(write).toHaveBeenCalledTimes(2);
  write.mockRestore();
});

test("on macOS Cmd+C copies and Cmd+V pastes, and Ctrl+C and Ctrl+Shift+V reach the shell", async () => {
  asMac();
  const write = spyOn(transport, "writeTerminal");
  const copy = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const read = spyOn(navigator.clipboard, "readText").mockResolvedValue("pasted");
  const { id, term } = await open();
  showTerminal(id);
  await new Promise<void>((resolve) => term.write("hello world", resolve));
  term.select(0, 0, 5);
  const cmd = { ctrlKey: false, metaKey: true, shiftKey: false };

  expect(press(term, "c", cmd).defaultPrevented).toBe(true);
  expect(copy).toHaveBeenCalledWith("hello");
  expect(press(term, "v", cmd).defaultPrevented).toBe(true);
  await settle();
  expect(write.mock.calls).toEqual([[id, "pasted"]]);

  press(term, "c", { shiftKey: false, keyCode: 67 }); // Ctrl+C
  expect(write).toHaveBeenLastCalledWith(id, "\x03");
  expect(press(term, "V", { keyCode: 86 }).defaultPrevented).toBe(false); // Ctrl+Shift+V
  press(term, "C", { ...cmd, shiftKey: true }); // Cmd+Shift+C: not the copy chord
  expect(copy).toHaveBeenCalledTimes(1);
  expect(read).toHaveBeenCalledTimes(1);
  copy.mockRestore();
  read.mockRestore();
  write.mockRestore();
});

test("closing ends a running terminal, drops it and its tab; an exited one is just dropped", async () => {
  const close = spyOn(transport, "closeTerminal");
  const one = await open();
  const two = await open();
  showTerminal(two.id);
  closeTerminal(two.id);
  expect(close).toHaveBeenCalledWith(two.id);
  expect(terminal(two.id)).toBeUndefined();
  expect(two.term.element?.isConnected).toBeFalsy();
  expect(useHive.getState().tabs.map((t) => t.id)).toEqual([one.id]);

  apply({ type: "terminal_exited", channel: one.id, code: 1 });
  closeTerminal(one.id);
  expect(close).toHaveBeenCalledTimes(1);
  expect(useHive.getState().tabs).toEqual([]);
  opened.length = 0;
  close.mockRestore();
});

test("split terminals both render with WebGL, left then right; showing one alone hides the other", async () => {
  const one = await open();
  const two = await open();
  const pane = (t: Terminal) => t.element?.parentElement as HTMLElement;
  showTerminals([two.id, one.id], two.id);
  expect([pane(two.term).dataset.pane, pane(one.term).dataset.pane]).toEqual(["left", "right"]);
  expect([pane(one.term).hidden, pane(two.term).hidden]).toEqual([false, false]);
  expect(addons.map((a) => a.disposed)).toEqual([false, false]);
  expect(document.activeElement).toBe(two.term.textarea as Element);

  showTerminal(one.id);
  expect(pane(one.term).dataset.pane).toBe("");
  expect(pane(two.term).hidden).toBe(true);
  // The one still shown keeps its renderer.
  expect(addons.map((a) => a.disposed)).toEqual([true, false]);
});

test("Ctrl+Shift+D splits with the next tab of the worktree, again un-splits; a click focuses a pane", async () => {
  await open("/elsewhere");
  const one = await open();
  const two = await open();
  // The next tab after the last one of the worktree wraps to its first, skipping other places.
  await splitTerminal(two.id);
  expect(useHive.getState().split).toEqual({ left: two.id, right: one.id });
  expect(shownTerminals(useHive.getState())).toEqual([two.id, one.id]);
  expect(useHive.getState().activeTab).toBe(one.id);

  // Clicking into the left pane makes it the active tab, the one in view.
  showTerminals([two.id, one.id], one.id);
  const focusIn = (t: Terminal) =>
    t.element?.parentElement?.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
  focusIn(two.term);
  expect(useHive.getState().activeTab).toBe(two.id);

  await splitTerminal(one.id);
  expect(useHive.getState().split).toBeNull();
  expect(shownTerminals(useHive.getState())).toEqual([two.id]);
  // Without a split, a pane taking the focus changes nothing.
  focusIn(one.term);
  expect(useHive.getState().activeTab).toBe(two.id);
  await splitTerminal(999);
  expect(useHive.getState().split).toBeNull();
  await splitTerminal(null);
  expect(useHive.getState().split).toBeNull();
});

test("a tab alone in its worktree splits beside a new terminal there; closing a pane un-splits", async () => {
  const one = await open();
  await splitTerminal(one.id);
  const { split, tabs, activeTab } = useHive.getState();
  const right = split?.right as number;
  opened.push(right);
  expect(split?.left).toBe(one.id);
  expect(tabs.map((t) => t.cwd)).toEqual(["/w", "/w"]);
  expect(activeTab).toBe(right);

  // Another tab shown alone leaves the split, which comes back with either of its panes.
  const three = await open();
  expect(shownTerminals(useHive.getState())).toEqual([three.id]);
  useHive.setState({ activeTab: one.id });
  expect(shownTerminals(useHive.getState())).toEqual([one.id, right]);

  // Closing the focused pane shows the other one alone.
  closeTerminal(one.id);
  expect(useHive.getState()).toMatchObject({ split: null, activeTab: right });
  opened.splice(opened.indexOf(one.id), 1);
});

test("moving the divider refits the shown panes, debounced", async () => {
  const fit = spyOn(FitAddon.prototype, "fit");
  const one = await open();
  const two = await open();
  showTerminals([one.id, two.id], one.id);
  fit.mockClear();
  setWidth("split", 30);
  setWidth("split", 40);
  await new Promise((resolve) => setTimeout(resolve, 80));
  expect(fit).toHaveBeenCalledTimes(2);
  fit.mockRestore();
});
