import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, createEvent, fireEvent, render, waitFor } from "@testing-library/react";
import { asMac } from "../test/mac";
import { App } from "./App";
import type { AgentState, Project } from "./protocol";
import { apply } from "./reduce";
import { paletteCommands } from "./shell/Palette";
import { nextPending, shortcut } from "./shortcuts";
import { initialState, openModal, renderedShown, select, setOpenFile, useHive } from "./store";
import { closeTerminal, openTerminal, terminal } from "./terminals";
import { transport } from "./transport";
import { agentStatus, MOCK_GROUP, MOCK_REPOS } from "./transport/mock";

beforeAll(async () => {
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;

function app() {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => apply({ type: "projects", projects: [shop, api] }));
}

type Keys = {
  key: string;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
};
const ctrlShift = (key: string): Keys => ({ key, ctrlKey: true, shiftKey: true });
/** Presses keys with the focus outside any terminal; true when the app took them. */
const press = (keys: Keys, target: Element = document.body) => {
  let taken = false;
  act(() => {
    taken = !fireEvent.keyDown(target, keys);
  });
  return taken;
};

test("Ctrl+Shift+T opens the worktree picker", () => {
  app();
  expect(press(ctrlShift("T"))).toBe(true);
  expect(useHive.getState().modal).toBe("worktree-picker");
});

test("Ctrl+Shift+P opens the command palette", () => {
  app();
  expect(press(ctrlShift("P"))).toBe(true);
  expect(useHive.getState().modal).toBe("palette");
});

test("Esc then Ctrl+Shift+P at once opens the palette, before the dialog's `close` event", () => {
  app();
  press({ key: ",", ctrlKey: true });
  const settings = document.querySelector("dialog") as HTMLDialogElement;
  expect(useHive.getState().modal).toBe("settings");
  // A dialog open in the store and on screen keeps the shortcuts.
  expect(press(ctrlShift("P"))).toBe(false);
  // Esc in the browser: the dialog is closed now, its `close` event comes in a later task.
  settings.removeAttribute("open");
  expect(useHive.getState().modal).toBe("settings");
  expect(press(ctrlShift("P"))).toBe(true);
  expect(useHive.getState().modal).toBe("palette");
});

test("Ctrl+Shift+N opens the new worktree dialog for the current project", () => {
  app();
  // Nothing selected: the first project.
  press(ctrlShift("N"));
  expect(useHive.getState()).toMatchObject({ modal: "new-worktree", modalProject: shop.id });
  // A selected project, or the project of a selected worktree.
  for (const selection of [api.id, api.worktrees[1].id]) {
    act(() => openModal(null));
    act(() => select(selection));
    press(ctrlShift("n"));
    expect(useHive.getState()).toMatchObject({ modal: "new-worktree", modalProject: api.id });
  }
  // A selected agent (F8): the project of its worktree.
  act(() => {
    openModal(null);
    const worktree = api.worktrees[1].id;
    apply({ type: "agent_detected", channel: 9, id: "s", project: api.id, worktree, cwd: null });
    select("s");
  });
  press(ctrlShift("N"));
  expect(useHive.getState()).toMatchObject({ modal: "new-worktree", modalProject: api.id });
});

test("a group has no new worktree; F8 to an agent of one of its projects opens the group", () => {
  const [group, backend] = MOCK_GROUP as [Project, Project];
  render(<App />);
  act(() => apply({ type: "projects", projects: MOCK_GROUP }));
  act(() => select(group.id));
  press(ctrlShift("N"));
  expect(useHive.getState().modal).toBeNull();
  act(() => {
    useHive.setState({ collapsed: { [group.id]: true } });
    const placed = { project: backend.id, worktree: backend.id, cwd: null };
    apply({ type: "agent_detected", channel: 9, id: "s", ...placed });
    apply({ type: "agent_state", id: "s", ...agentStatus("waiting_you"), subagents: [] });
  });
  press({ key: "F8" });
  expect(useHive.getState().selection).toBe("s");
  expect(useHive.getState().collapsed[group.id]).toBe(false);
});

test("Ctrl+Shift+N without projects opens the add project dialog", () => {
  render(<App />);
  act(() => apply({ type: "projects", projects: [] }));
  press(ctrlShift("N"));
  expect(useHive.getState().modal).toBe("add-project");
});

test("Ctrl+Shift+B toggles the files panel and Ctrl+Shift+O opens add project", () => {
  app();
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBeNull();
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBe("files");
  press(ctrlShift("O"));
  expect(useHive.getState().modal).toBe("add-project");
});

test("F8 does nothing while no agent is pending", () => {
  app();
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8" }))).toBe(nextPending);
  act(() => {
    apply({
      type: "agent_detected",
      channel: 1,
      id: "s1",
      project: null,
      worktree: null,
      cwd: null,
    });
    apply({ type: "agent_state", id: "s1", ...agentStatus("working"), subagents: [] });
  });
  const before = useHive.getState();
  expect(press({ key: "F8" })).toBe(true);
  expect(useHive.getState()).toBe(before);
});

test("F8 cycles through pending agents in tree order, revealing and showing each", () => {
  app();
  const agent = (id: string, terminal: number, worktree: string | null, state: AgentState) => {
    const project = [shop, api].find((p) => p.worktrees.some((w) => w.id === worktree));
    const placed = { project: project?.id ?? null, worktree, cwd: worktree };
    apply({ type: "agent_detected", channel: terminal, id, ...placed });
    apply({ type: "agent_state", id, ...agentStatus(state), subagents: [] });
  };
  const fixLogin = shop.worktrees[1].id;
  act(() => {
    useHive.setState({
      tabs: [
        { id: 1, cwd: fixLogin },
        { id: 2, cwd: shop.path },
      ],
      collapsed: { [shop.id]: true, [api.id]: true, [`worktree:${api.worktrees[0].id}`]: true },
    });
    // Arrival order is not tree order.
    agent("s1", 1, fixLogin, "waiting_you");
    agent("s2", 5, api.worktrees[0].id, "error");
    agent("s3", 2, shop.path, "waiting_permission");
    agent("s4", 3, null, "waiting_you");
    agent("s5", 4, api.worktrees[1].id, "idle");
  });
  const step = () => {
    press({ key: "F8" });
    const { selection, activeTab } = useHive.getState();
    return [selection, activeTab];
  };
  expect(step()).toEqual(["s3", 2]);
  expect(useHive.getState().collapsed[shop.id]).toBe(false);
  expect(step()).toEqual(["s1", 1]);
  // No tab: the shown terminal stays.
  expect(step()).toEqual(["s2", 1]);
  expect(useHive.getState().collapsed).toMatchObject({
    [api.id]: false,
    [`worktree:${api.worktrees[0].id}`]: false,
  });
  expect(step()).toEqual(["s4", 1]);
  expect(step()).toEqual(["s3", 2]);
  // With a worktree selected, the agent whose terminal is shown is the current one.
  act(() => {
    select(fixLogin);
    useHive.setState({ activeTab: 1 });
  });
  expect(step()).toEqual(["s2", 1]);
});

test("other keys are not shortcuts", () => {
  app();
  for (const keys of [
    { key: "T", ctrlKey: true },
    { key: "O", ctrlKey: true },
    { key: "b", altKey: true },
    { key: "t" },
    { key: "F8", shiftKey: true },
    { key: "T", ctrlKey: true, shiftKey: true, altKey: true },
    { key: "T", ctrlKey: true, shiftKey: true, metaKey: true },
    { key: "T", metaKey: true, shiftKey: true },
    ctrlShift("C"),
    ctrlShift("V"),
    ctrlShift("X"),
  ]) {
    expect(press(keys)).toBe(false);
  }
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: "files" });
});

test("on macOS the shortcuts are Cmd+Shift+letter and Ctrl+Shift+letter is the terminal's", () => {
  asMac();
  app();
  expect(press(ctrlShift("T"))).toBe(false);
  expect(press({ key: "T", ctrlKey: true, metaKey: true, shiftKey: true })).toBe(false);
  expect(useHive.getState().modal).toBeNull();
  expect(press({ key: "t", metaKey: true, shiftKey: true })).toBe(true);
  expect(useHive.getState().modal).toBe("worktree-picker");
});

test("Ctrl+, (Cmd+, on macOS) opens the settings; with Shift or Alt it is not a shortcut", () => {
  app();
  expect(press({ key: ",", ctrlKey: true, shiftKey: true })).toBe(false);
  expect(press({ key: ",", ctrlKey: true, altKey: true })).toBe(false);
  expect(press({ key: "," })).toBe(false);
  expect(useHive.getState().modal).toBeNull();
  expect(press({ key: ",", ctrlKey: true })).toBe(true);
  expect(useHive.getState().modal).toBe("settings");
  act(() => openModal(null));
  asMac();
  expect(press({ key: ",", ctrlKey: true })).toBe(false);
  expect(press({ key: ",", metaKey: true })).toBe(true);
  expect(useHive.getState().modal).toBe("settings");
});

test("F8 with a modifier is not a shortcut", () => {
  app();
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8", metaKey: true }))).toBeNull();
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8", shiftKey: true }))).toBeNull();
});

test("nothing runs under the connection block or while a dialog is open", () => {
  app();
  act(() => openModal("add-project"));
  expect(press(ctrlShift("B"))).toBe(false);
  act(() => openModal(null));
  for (const block of [
    { type: "disconnected", reason: "gone", bundled: false } as const,
    {
      type: "version_mismatch",
      protocol: 2,
      version: "0.2.0",
      app_protocol: 1,
      app_version: "0.1.0",
      bundled: false,
    } as const,
  ]) {
    act(() => apply(block));
    expect(press(ctrlShift("B"))).toBe(false);
    expect(press(ctrlShift("T"))).toBe(false);
    expect(press({ key: "F8" })).toBe(false);
  }
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: "files" });
});

test("in a terminal, shortcuts run once and never reach it; other keys do", async () => {
  const write = spyOn(transport, "writeTerminal");
  app();
  const id = await act(() => openTerminal(shop.worktrees[1].path));
  const textarea = terminal(id)?.textarea as HTMLTextAreaElement;
  press(ctrlShift("B"), textarea);
  expect(useHive.getState().rightPanel).toBeNull();
  press({ key: "F8", code: "F8", keyCode: 119 } as Keys, textarea);
  expect(write).not.toHaveBeenCalled();
  // Ctrl+O is Claude Code's: it goes to the terminal.
  press({ key: "o", code: "KeyO", keyCode: 79, ctrlKey: true } as Keys, textarea);
  await waitFor(() => expect(write).toHaveBeenCalledWith(id, "\x0f"));
  expect(useHive.getState()).toMatchObject({ modal: null, rightPanel: null });
  write.mockRestore();
});

test("Ctrl+Shift+L writes the selected lines' reference into the terminal and focuses it", async () => {
  const write = spyOn(transport, "writeTerminal").mockImplementation(async () => {});
  app();
  const worktree = shop.worktrees[1].path;
  const id = await act(() => openTerminal(worktree));
  // Lets the mock's first messages for the new terminal arrive.
  await act(() => new Promise((resolve) => setTimeout(resolve, 10)));
  act(() => {
    useHive.setState({
      openFile: { worktree, path: "src/auth/session.ts" },
      selectedLines: { from: 12, to: 12 },
    });
    (document.activeElement as HTMLElement | null)?.blur();
  });
  expect(press(ctrlShift("L"))).toBe(true);
  expect(write).toHaveBeenCalledWith(id, "@src/auth/session.ts (line 12) ");
  expect(document.activeElement).toBe(terminal(id)?.textarea as Element);
  write.mockRestore();
});

test("Ctrl+Shift+M opens the comment input for the selected lines of the shown file", () => {
  app();
  const openFile = { worktree: shop.worktrees[1].path, path: "src/auth/session.ts" };
  act(() => useHive.setState({ openFile, fileShown: true, selectedLines: { from: 4, to: 6 } }));
  expect(press(ctrlShift("M"))).toBe(true);
  expect(useHive.getState().commenting).toEqual({ ...openFile, from: 4, to: 6 });
});

test("the app stops listening when it unmounts", () => {
  app();
  cleanup();
  press(ctrlShift("B"));
  expect(useHive.getState().rightPanel).toBe("files");
});

test("the WebView's context menu is off except in text fields and the editor", () => {
  app();
  // fireEvent returns false when the default was prevented.
  const menu = (target: Element) => fireEvent.contextMenu(target);
  const editor = document.createElement("div");
  editor.className = "cm-editor";
  const line = editor.appendChild(document.createElement("span"));
  const input = document.createElement("input");
  const textarea = document.createElement("textarea");
  document.body.append(editor, input, textarea);
  expect(menu(document.body)).toBe(false);
  expect(menu(line)).toBe(true);
  expect(menu(input)).toBe(true);
  expect(menu(textarea)).toBe(true);
  for (const node of [editor, input, textarea]) node.remove();
  // Uninstalled with the app.
  cleanup();
  expect(menu(document.body)).toBe(true);
});

test("a file or link from outside that nothing takes is refused: the WebView would open it", () => {
  app();
  const taken = document.body.appendChild(document.createElement("div"));
  taken.addEventListener("drop", (e) => e.preventDefault());
  // `fire` returns the event after it went through the window.
  const fire = (target: Element, kind: "dragOver" | "drop", types: string[]) => {
    const dataTransfer = { types, dropEffect: "copy" };
    const event = createEvent[kind](target, { dataTransfer }) as DragEvent;
    fireEvent(target, event);
    return [event.defaultPrevented, event.dataTransfer?.dropEffect];
  };
  expect(fire(document.body, "dragOver", ["Files"])).toEqual([true, "none"]);
  expect(fire(document.body, "drop", ["text/uri-list", "text/plain"])).toEqual([true, "none"]);
  // The app's own drags are left to their targets; one taken on the way keeps its effect.
  expect(fire(document.body, "dragOver", ["text/plain"])).toEqual([false, "copy"]);
  expect(fireEvent.dragOver(document.body)).toBe(true);
  expect(fire(taken, "drop", ["Files"])).toEqual([true, "copy"]);
  taken.remove();
  cleanup();
  expect(fire(document.body, "drop", ["Files"])).toEqual([false, "copy"]);
});

test("Ctrl+Shift+D splits the active terminal and pressed again un-splits", async () => {
  app();
  const ids: number[] = [];
  const cwd = shop.worktrees[1]?.path as string;
  await act(async () => {
    ids.push(await openTerminal(cwd), await openTerminal(cwd));
  });
  expect(press(ctrlShift("D"))).toBe(true);
  expect(useHive.getState().split).toEqual({ left: ids[1] as number, right: ids[0] as number });
  press(ctrlShift("D"));
  expect(useHive.getState().split).toBeNull();
});

test("Ctrl+Shift+V (Cmd+Shift+V on macOS) toggles a Markdown file's tab, elsewhere the terminal pastes", async () => {
  app();
  let id = 0;
  await act(async () => {
    id = await openTerminal(api.path);
  });
  const toggle = "Show a Markdown file rendered or as text";
  const offered = () => paletteCommands(useHive.getState()).some((c) => c.label === toggle);
  act(() => setOpenFile({ worktree: api.path, path: "README.md" }));
  expect(offered()).toBe(true);
  expect(press(ctrlShift("V"))).toBe(true);
  expect(renderedShown(useHive.getState())).toBe(true);
  expect(press(ctrlShift("V"))).toBe(true);
  expect(renderedShown(useHive.getState())).toBe(false);

  act(() => setOpenFile({ worktree: api.path, path: "package.json" }));
  expect(press(ctrlShift("V"))).toBe(false);
  expect(offered()).toBe(false);

  // The terminal shown, a Markdown file's tab behind it: Ctrl+Shift+V is the terminal's paste.
  act(() => setOpenFile({ worktree: api.path, path: "README.md" }));
  act(() => useHive.setState({ fileShown: false, activeTab: id }));
  expect(offered()).toBe(false);
  const read = spyOn(navigator.clipboard, "readText").mockResolvedValue("pasted");
  const paste = new KeyboardEvent("keydown", {
    key: "V",
    ctrlKey: true,
    shiftKey: true,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    terminal(id)?.textarea?.dispatchEvent(paste);
  });
  expect(read).toHaveBeenCalledTimes(1);
  expect(renderedShown(useHive.getState())).toBe(false);
  read.mockRestore();

  asMac();
  act(() => useHive.setState({ fileShown: true }));
  expect(press(ctrlShift("V"))).toBe(false);
  expect(press({ key: "v", metaKey: true, shiftKey: true })).toBe(true);
  expect(renderedShown(useHive.getState())).toBe(true);
});

test("on macOS ⇧⌘J goes to the next pending agent too (F8 is Play/Pause there); elsewhere it is the terminal's", () => {
  app();
  expect(shortcut(new KeyboardEvent("keydown", ctrlShift("J")))).toBeNull();
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8" }))).toBe(nextPending);
  asMac();
  expect(shortcut(new KeyboardEvent("keydown", { key: "j", metaKey: true, shiftKey: true }))).toBe(
    nextPending,
  );
  expect(shortcut(new KeyboardEvent("keydown", { key: "F8" }))).toBe(nextPending);
  expect(shortcut(new KeyboardEvent("keydown", ctrlShift("J")))).toBeNull();
  expect(shortcut(new KeyboardEvent("keydown", { key: "j", metaKey: true }))).toBeNull();
});

/** Two terminals and a file open in fix-login: the bar holds them in that order. */
async function threeTabs(): Promise<number[]> {
  app();
  const worktree = shop.worktrees[1].path;
  const ids: number[] = [];
  await act(async () => {
    ids.push(await openTerminal(worktree), await openTerminal(worktree));
  });
  act(() => setOpenFile({ worktree, path: "src/a.ts" }));
  return ids;
}

const shownTab = () => {
  const s = useHive.getState();
  return s.fileShown ? s.openFile?.path : s.activeTab;
};

test("macOS' tab keys: ⌘1–9 show a tab, ⇧⌘] and ⇧⌘[ the next and previous, wrapping; not elsewhere", async () => {
  const [one, two] = await threeTabs();
  for (const keys of [
    { key: "1", ctrlKey: true },
    { key: "}", ctrlKey: true, shiftKey: true },
    { key: "w", ctrlKey: true },
    { key: "=", ctrlKey: true },
  ]) {
    expect(press(keys)).toBe(false);
  }
  expect(shownTab()).toBe("src/a.ts");

  asMac();
  expect(press({ key: "1", metaKey: true })).toBe(true);
  expect(shownTab()).toBe(one);
  expect(press({ key: "3", metaKey: true })).toBe(true);
  expect(shownTab()).toBe("src/a.ts");
  // A layout whose digits need Shift (AZERTY's "&" is 1): the key's place counts.
  expect(press({ key: "&", code: "Digit2", metaKey: true } as Keys)).toBe(true);
  expect(shownTab()).toBe(two);
  // No fourth tab: taken, nothing changes.
  expect(press({ key: "4", metaKey: true })).toBe(true);
  expect(shownTab()).toBe(two);
  // ⇧⌘] types "}" on a US layout: matched by its place too.
  expect(press({ key: "}", code: "BracketRight", metaKey: true, shiftKey: true } as Keys)).toBe(
    true,
  );
  expect(shownTab()).toBe("src/a.ts");
  press({ key: "]", metaKey: true, shiftKey: true });
  expect(shownTab()).toBe(one);
  press({ key: "{", code: "BracketLeft", metaKey: true, shiftKey: true } as Keys);
  expect(shownTab()).toBe("src/a.ts");
  press({ key: "[", metaKey: true, shiftKey: true });
  expect(shownTab()).toBe(two);
  // Without Shift, ⌘] is not a tab key.
  expect(press({ key: "]", code: "BracketRight", metaKey: true } as Keys)).toBe(false);
});

test("⇧⌘] with no tab does nothing", () => {
  asMac();
  app();
  const before = useHive.getState();
  expect(press({ key: "]", metaKey: true, shiftKey: true })).toBe(true);
  expect(useHive.getState()).toBe(before);
});

test("⌘W closes the tab shown: a file, then a terminal; with none, nothing", async () => {
  const close = spyOn(transport, "closeTerminal");
  const [one, two] = await threeTabs();
  asMac();
  expect(press({ key: "w", metaKey: true })).toBe(true);
  expect(useHive.getState().openFiles).toEqual([]);
  expect(shownTab()).toBe(two);
  press({ key: "w", metaKey: true });
  expect(close.mock.calls).toEqual([[two as number]]);
  expect(useHive.getState().tabs.map((t) => t.id)).toEqual([one as number]);
  press({ key: "w", metaKey: true });
  expect(useHive.getState().tabs).toEqual([]);
  const before = useHive.getState();
  press({ key: "w", metaKey: true });
  expect(useHive.getState()).toBe(before);
  close.mockRestore();
});

test("⌘W leaves a file of another worktree alone when the bar does not show it", async () => {
  const [one] = await threeTabs();
  asMac();
  // Another worktree selected: its bar holds neither the file nor the terminals.
  act(() => select(shop.path));
  const before = useHive.getState();
  press({ key: "w", metaKey: true });
  expect(useHive.getState().openFiles).toEqual(before.openFiles);
  expect(useHive.getState().tabs.map((t) => t.id)).toContain(one as number);
});

test("⌘= and ⌘- change every terminal's font size, ⌘0 gives back the settings' one", async () => {
  const [one, two] = await threeTabs();
  asMac();
  const sizes = () => [one, two].map((id) => terminal(id as number)?.options.fontSize);
  expect(press({ key: "=", metaKey: true })).toBe(true);
  expect(sizes()).toEqual([14, 14]);
  press({ key: "-", metaKey: true });
  press({ key: "-", metaKey: true });
  expect(sizes()).toEqual([12, 12]);
  press({ key: "0", metaKey: true });
  expect(sizes()).toEqual([13, 13]);
});
