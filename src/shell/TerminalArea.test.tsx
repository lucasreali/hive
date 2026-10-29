import { afterEach, expect, spyOn, test } from "bun:test";
import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import { asMac } from "../../test/mac";
import { App } from "../App";
import type { ProgressState } from "../protocol";
import { apply } from "../reduce";
import {
  addTab,
  initialState,
  select,
  setEdit,
  setOpenFile,
  setSplit,
  unsplit,
  useHive,
} from "../store";
import * as tabsModule from "../tabs";
import { closeTerminal, terminal } from "../terminals";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";
import { toText } from "../viewer/buffer";

const [shop, api] = MOCK_REPOS;
const [shopMain, fixLogin] = shop.worktrees;
const [apiMain] = api.worktrees;

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

function show() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
}

const bar = () => within(screen.getByRole("tablist", { name: "Open terminals and files" }));
const tab = (name: string) => bar().getByRole("tab", { name: new RegExp(`^${name}`) });
const plus = () => screen.getByRole("button", { name: "New terminal or file" });
/** Clicks "+" and the menu's `item`. */
function newTab(item: string) {
  fireEvent.click(plus());
  fireEvent.click(screen.getByRole("menuitem", { name: item }));
}

test("the + menu opens a new file in the selected worktree", () => {
  show();
  fireEvent.click(screen.getByRole("button", { name: "fix-login" }));
  expect(plus().getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(plus());
  expect(plus().getAttribute("aria-expanded")).toBe("true");
  const menu = screen.getByRole("menu", { name: "New tab" });
  expect(
    within(menu)
      .getAllByRole("menuitem")
      .map((i) => i.textContent),
  ).toEqual(["Terminal", "New file…"]);
  expect(menu.querySelectorAll("svg[aria-hidden=true]").length).toBe(2);
  // "+" again closes it.
  fireEvent.click(plus());
  expect(screen.queryByRole("menu")).toBeNull();

  newTab("New file…");
  expect(screen.queryByRole("menu")).toBeNull();
  expect(useHive.getState()).toMatchObject({
    modal: "file-name",
    fileDialog: { worktree: fixLogin.id, folder: "", path: null, kind: "file" },
  });
});

test("the + menu from the keyboard: down opens it, arrows move, Esc gives the focus back", () => {
  show();
  fireEvent.click(screen.getByRole("button", { name: "fix-login" }));
  plus().focus();
  fireEvent.keyDown(plus(), { key: "ArrowDown" });
  const item = (name: string) => screen.getByRole("menuitem", { name });
  expect(document.activeElement).toBe(item("Terminal"));
  fireEvent.keyDown(item("Terminal"), { key: "ArrowDown" });
  expect(document.activeElement).toBe(item("New file…"));
  fireEvent.keyDown(item("New file…"), { key: "Escape" });
  expect(screen.queryByRole("menu")).toBeNull();
  expect(document.activeElement).toBe(plus());
});

test("New terminal opens one in the selected worktree, shown in its tab", async () => {
  const open = spyOn(transport, "openTerminal");
  show();
  const button = screen.getByTitle("New terminal or file") as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "fix-login" }));
  newTab("Terminal");
  expect(open.mock.calls[0][0]).toBe(fixLogin.path);
  await waitFor(() => expect(tab("fix-login").getAttribute("aria-selected")).toBe("true"));
  expect(tab("fix-login").closest(".tab")?.getAttribute("title")).toBe(fixLogin.path);
  open.mockRestore();
});

test("with an agent selected (F8), New terminal opens one in the agent's worktree", () => {
  const open = spyOn(transport, "openTerminal");
  show();
  const button = screen.getByTitle("New terminal or file") as HTMLButtonElement;
  const agent = { type: "agent_detected", channel: 9, cwd: "/x" } as const;
  act(() => {
    apply({ ...agent, id: "s", project: shop.id, worktree: fixLogin.id });
    select("s");
  });
  newTab("Terminal");
  expect(open.mock.calls.map((call) => call[0])).toEqual([fixLogin.path]);
  // An agent outside every project has no worktree to open one in.
  act(() => {
    apply({ ...agent, id: "out", project: null, worktree: null });
    select("out");
  });
  expect(button.disabled).toBe(true);
  open.mockRestore();
});

test("a store update builds the tab bar once, and the tabs keep their order and keys (19.17)", () => {
  show();
  act(() => {
    addTab(1, shopMain.path);
    addTab(2, shopMain.path);
  });
  const built = spyOn(tabsModule, "barItems");
  try {
    act(() => useHive.setState({ focused: !useHive.getState().focused }));
    // React reads the one selector four times per update here (useSyncExternalStore's checks);
    // a second selector building the bar again made it seven.
    expect(built).toHaveBeenCalledTimes(4);
  } finally {
    built.mockRestore();
  }
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["main", "main"]);
});

test("tabs switch and show exit and missing hooks", () => {
  show();
  act(() => {
    addTab(1, shopMain.path);
    addTab(2, apiMain.path);
    addTab(3, "/somewhere/else");
  });
  // Only the selected worktree's tabs show: the last one opened selected its own place.
  const tabs = () => bar().getAllByRole("tab");
  expect(tabs().map((t) => t.textContent)).toEqual(["/somewhere/else"]);
  // With nothing selected every tab shows, by its worktree's name (no project: a tab bar
  // shows one worktree's tabs).
  act(() => select(null));
  expect(tabs().map((t) => t.textContent)).toEqual(["main", "main", "/somewhere/else"]);
  expect(tabs().map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "false", "true"]);

  fireEvent.click(tabs()[0]);
  expect(tabs().map((t) => t.getAttribute("aria-selected"))).toEqual(["true"]);
  expect(useHive.getState().selection).toBe(shopMain.path);
  act(() => select(null));

  act(() => apply({ type: "badge", channel: 2, text: "db" }));
  expect(tabs()[1].textContent).toBe("maindb");
  act(() => apply({ type: "unhooked_agent", channel: 1 }));
  act(() => apply({ type: "terminal_exited", channel: 2, code: 3 }));
  act(() => apply({ type: "terminal_exited", channel: 3, code: null }));
  expect(screen.getByText("no hooks").getAttribute("title")).toBe(
    "Claude runs in this terminal without Hive's hooks: its state is not observed",
  );
  expect(tabs()[1].textContent).toBe("mainexited");
  expect(screen.getAllByText("exited").map((b) => b.getAttribute("title"))).toEqual([
    "Exit code: 3",
    "Exit code: none (killed)",
  ]);
});

test("a program's progress shows on its tab until it clears or the terminal exits (17.25)", () => {
  show();
  act(() => addTab(1, shopMain.path));
  const bar1 = () => screen.queryByRole("progressbar");
  const progress = (state: ProgressState, value = 0) =>
    act(() => apply({ type: "terminal_progress", channel: 1, state, value }));
  expect(bar1()).toBeNull();
  const seen: (string | null)[][] = [];
  for (const [state, value] of [
    ["normal", 40],
    ["error", 70],
    ["paused", 10],
    ["indeterminate", 0],
  ] as const) {
    progress(state, value);
    const hint = bar1() as HTMLElement;
    seen.push([
      hint.getAttribute("aria-label"),
      hint.getAttribute("aria-valuenow"),
      hint.dataset.state ?? null,
      hint.style.getPropertyValue("--progress"),
    ]);
  }
  expect(seen).toEqual([
    ["Progress 40%", "40", "normal", "40%"],
    ["Failed at 70%", "70", "error", "70%"],
    ["Paused at 10%", "10", "paused", "10%"],
    ["Busy", null, "indeterminate", "0%"],
  ]);
  progress("none");
  expect(bar1()).toBeNull();
  progress("normal", 5);
  act(() => apply({ type: "terminal_exited", channel: 1, code: 0 }));
  expect(bar1()).toBeNull();
});

test("a tab running Claude shows the agent's state and its session's name", () => {
  show();
  act(() => addTab(4, fixLogin.path));
  const tab = () => bar().getByRole("tab");
  expect(tab().querySelector(".tab-name")?.textContent).toBe("fix-login");
  const agent = { type: "agent_detected", channel: 4, id: "s", project: shop.id } as const;
  act(() => apply({ ...agent, worktree: fixLogin.id, cwd: fixLogin.path }));
  // Idle until its first state; the worktree's name until the session has one.
  expect(tab().querySelector("[role=img]")?.getAttribute("aria-label")).toBe("idle");
  act(() => {
    apply({
      type: "agent_state",
      id: "s",
      state: "working",
      urgency: 2,
      pending: false,
      interrupted: false,
      alert: null,
      notify: false,
      writing: true,
      subagents: [],
      activity: null,
      since_ms: 0,
    });
    apply({ type: "agent_title", channel: 4, id: "s", title: "Fix the login redirect" });
  });
  expect(tab().querySelector("[role=img]")?.getAttribute("aria-label")).toBe("working");
  // A 🟠 already seen (not pending) is drawn muted, as in the sidebar (15.6).
  const waiting = { type: "agent_state", id: "s", ...agentStatus("waiting_you"), subagents: [] };
  act(() => apply({ ...waiting, type: "agent_state", pending: false }));
  expect(tab().querySelector("[role=img]")?.getAttribute("aria-label")).toBe(
    "waiting for you, seen",
  );
  act(() => apply({ ...waiting, type: "agent_state" }));
  expect(tab().querySelector("[role=img]")?.hasAttribute("data-muted")).toBe(false);
  expect(tab().querySelector(".tab-name")?.textContent).toBe("Fix the login redirect");
  expect(tab().closest(".tab")?.getAttribute("title")).toBe(
    `Fix the login redirect\n${fixLogin.path}`,
  );
  expect(
    screen.getByRole("button", { name: "Close terminal Fix the login redirect" }),
  ).toBeDefined();
  // The session ended: the tab is the worktree's again.
  act(() => apply({ type: "agent_removed", channel: 4, id: "s" }));
  expect(tab().querySelector(".tab-name")?.textContent).toBe("fix-login");
  expect(useHive.getState().agentTitles).toEqual({});
});

test("the close button ends the terminal and removes its tab", () => {
  const close = spyOn(transport, "closeTerminal");
  show();
  act(() => addTab(7, fixLogin.path));
  fireEvent.click(screen.getByRole("button", { name: "Close terminal fix-login" }));
  expect(close).toHaveBeenCalledWith(7);
  expect(bar().queryByRole("tab")).toBeNull();
  close.mockRestore();
});

test("the open file has its tab after the terminals, shown in place of the terminal", () => {
  show();
  act(() => addTab(1, shopMain.path));
  act(() => setOpenFile({ worktree: shopMain.path, path: "src/app.ts" }));
  const host = () => document.querySelector(".terminal-host") as HTMLElement;
  const tabs = () =>
    bar()
      .getAllByRole("tab")
      .map((t) => t.getAttribute("aria-selected"));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["main", "app.ts"]);
  // Opened, it is the preview tab (11.1): its name in italics, and said so; a double click keeps it.
  const name = () => tab("app.ts").querySelector(".tab-name") as HTMLElement;
  expect(tab("app.ts").closest(".tab")?.getAttribute("title")).toBe(
    "src/app.ts\nPreview: opening another file replaces it",
  );
  expect(tab("app.ts").getAttribute("aria-description")).toBe("Preview");
  expect(name().hasAttribute("data-preview")).toBe(true);
  fireEvent.doubleClick(tab("app.ts"));
  expect(tab("app.ts").closest(".tab")?.getAttribute("title")).toBe("src/app.ts");
  expect(tab("app.ts").hasAttribute("aria-description")).toBe(false);
  expect(name().hasAttribute("data-preview")).toBe(false);
  expect(tabs()).toEqual(["false", "true"]);
  expect(host().hidden).toBe(true);
  expect(screen.getByRole("region", { name: "src/app.ts" })).toBeDefined();

  fireEvent.click(tab("main"));
  expect(tabs()).toEqual(["true", "false"]);
  expect(host().hidden).toBe(false);
  expect(screen.queryByRole("region", { name: "src/app.ts" })).toBeNull();

  fireEvent.click(tab("app.ts"));
  expect(tabs()).toEqual(["false", "true"]);
  // A terminal in another worktree selects it: only its tab shows, the file's goes with main.
  act(() => addTab(2, fixLogin.path));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["fix-login"]);
  expect(screen.queryByRole("region", { name: "src/app.ts" })).toBeNull();
  act(() => select(shopMain.id));
  expect(
    bar()
      .getAllByRole("tab")
      .map((t) => t.textContent),
  ).toEqual(["main", "app.ts"]);
  expect(tabs()).toEqual(["true", "false"]);
  // Leaving the worktree while its file is shown hides the file; coming back shows a terminal.
  fireEvent.click(tab("app.ts"));
  act(() => select(fixLogin.id));
  expect(useHive.getState().fileShown).toBe(false);
  act(() => select(shopMain.id));
  expect(tabs()).toEqual(["true", "false"]);
});

test("a worktree without terminals says so and opens one", () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(5);
  show();
  act(() => addTab(1, shopMain.path));
  act(() => select(fixLogin.id));
  expect(bar().queryByRole("tab")).toBeNull();
  expect(screen.getByText("No terminal in fix-login")).toBeDefined();
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "New terminal" }));
  expect(open.mock.calls[0]?.[0]).toBe(fixLogin.path);
  open.mockRestore();
});

test("unsaved edits put a dot in place of the file tab's ×, named for screen readers", () => {
  show();
  act(() => setOpenFile({ worktree: shopMain.path, path: "src/app.ts" }));
  const buffer = {
    worktree: shopMain.path,
    path: "src/app.ts",
    doc: toText("one\n"),
    saved: toText("one\n"),
    version: "v",
    conflict: null,
    saving: null,
    error: null,
    recheck: 0,
  };
  act(() => setEdit(buffer));
  const clean = screen.getByRole("button", { name: "Close file app.ts" });
  expect(clean.hasAttribute("data-dirty")).toBe(false);
  expect(clean.querySelector(".dirty")).toBeNull();

  act(() => setEdit({ ...buffer, doc: toText("two\n") }));
  const dirty = screen.getByRole("button", { name: "Close file app.ts (unsaved changes)" });
  expect(dirty.hasAttribute("data-dirty")).toBe(true);
  expect(dirty.querySelector(".dirty")).not.toBeNull();
  // The dot is on the × only, not a second marker in the label.
  expect(tab("app.ts").querySelector(".dirty")).toBeNull();
});

test("a tab's menu splits it right, marks both tabs and shows a divider; Unsplit and Close end it", async () => {
  const close = spyOn(transport, "closeTerminal");
  show();
  act(() => {
    addTab(1, fixLogin.path);
    addTab(2, fixLogin.path);
    addTab(3, fixLogin.path);
  });
  const menu = () => screen.getByRole("menu", { name: "Terminal" });
  const tabs = () => bar().getAllByRole("tab");
  fireEvent.contextMenu(tabs()[0] as HTMLElement);
  fireEvent.click(within(menu()).getByRole("menuitem", { name: "Split right" }));
  await waitFor(() => expect(useHive.getState().splits).toEqual([{ left: 1, right: 2 }]));
  expect(screen.queryByRole("menu")).toBeNull();
  // The focused pane's tab is the active one; the other pane's is marked beside it.
  expect(tabs().map((t) => t.closest(".tab")?.getAttribute("data-split") ?? null)).toEqual([
    "view",
    null,
    null,
  ]);
  expect(tabs()[1]?.getAttribute("aria-selected")).toBe("true");
  const divider = screen.getByRole("separator", { name: "Resize the split terminals" });
  expect(divider.getAttribute("aria-valuenow")).toBe("50");
  fireEvent.keyDown(divider, { key: "ArrowLeft" });
  expect(useHive.getState().splitPercent).toBe(48);
  const body = screen.getByRole("region", { name: "Terminals" }).querySelector(".terminal-body");
  expect((body as HTMLElement).style.getPropertyValue("--split")).toBe("0.48");

  fireEvent.contextMenu(tabs()[0] as HTMLElement);
  fireEvent.click(within(menu()).getByRole("menuitem", { name: "Unsplit" }));
  await waitFor(() => expect(useHive.getState().splits).toEqual([]));
  expect(screen.queryByRole("separator", { name: "Resize the split terminals" })).toBeNull();

  fireEvent.contextMenu(tabs()[2] as HTMLElement);
  fireEvent.click(within(menu()).getByRole("menuitem", { name: "Close terminal" }));
  expect(close).toHaveBeenCalledWith(3);
  expect(tabs()).toHaveLength(2);
  // Esc closes the menu without doing anything.
  fireEvent.contextMenu(tabs()[0] as HTMLElement);
  fireEvent.keyDown(menu(), { key: "Escape" });
  expect(screen.queryByRole("menu")).toBeNull();
  close.mockRestore();
});

test("the right panel's toggle keeps aria-pressed but opts out of the pressed look (8.17)", () => {
  show();
  const toggle = screen.getByTitle(/^Files, diff and sessions/);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  expect(toggle.classList.contains("unpressed-look")).toBe(true);
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
});

/** Opens a terminal in fix-login from the + menu; returns its id and xterm instance. */
async function openShown() {
  show();
  fireEvent.click(screen.getByRole("button", { name: "fix-login" }));
  newTab("Terminal");
  await waitFor(() => expect(tab("fix-login").getAttribute("aria-selected")).toBe("true"));
  const id = useHive.getState().activeTab as number;
  const inside = document.querySelector(`[data-terminal="${id}"] .xterm-screen`) as HTMLElement;
  return { id, term: terminal(id) as Terminal, inside };
}

const paneMenu = () => screen.getByRole("menu", { name: "Terminal" });
const paneItems = () =>
  within(paneMenu())
    .getAllByRole("menuitem")
    .map((i) => i.textContent);

test("a right click in a terminal opens its menu: Copy, Paste, Clear and Split, with their keys", async () => {
  const copy = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const read = spyOn(navigator.clipboard, "readText").mockResolvedValue("pasted");
  const write = spyOn(transport, "writeTerminal");
  const { id, term, inside } = await openShown();
  const clear = spyOn(term, "clear");
  const host = document.querySelector(".terminal-host") as HTMLElement;
  const item = (name: RegExp) => within(paneMenu()).getByRole("menuitem", { name });

  // The host around the panes has no menu (the window's handler blocks the WebView's).
  fireEvent.contextMenu(host);
  expect(screen.queryByRole("menu")).toBeNull();

  expect(fireEvent.contextMenu(inside, { clientX: 40, clientY: 50 })).toBe(false);
  expect(paneItems()).toEqual([
    "Copy Ctrl+Shift+C",
    "Paste Ctrl+Shift+V",
    "Clear",
    "Split right Ctrl+Shift+D",
  ]);
  expect(paneMenu().querySelectorAll("svg[aria-hidden=true]").length).toBe(4);
  // Nothing selected: Copy is disabled.
  expect((item(/^Copy/) as HTMLButtonElement).disabled).toBe(true);
  // The focus goes to the first item that can take it, so Esc and the arrows work.
  expect(document.activeElement).toBe(item(/^Paste/));
  fireEvent.click(item(/^Clear/));
  expect(clear).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("menu")).toBeNull();

  await new Promise<void>((resolve) => term.write("\r\nhello world", resolve));
  const row = term.buffer.active.cursorY;
  term.select(0, row, 5);
  fireEvent.contextMenu(inside);
  fireEvent.click(item(/^Copy/));
  expect(copy.mock.calls).toEqual([["hello"]]);

  fireEvent.contextMenu(inside);
  fireEvent.click(item(/^Paste/));
  await waitFor(() => expect(write).toHaveBeenCalledWith(id, "pasted"));
  expect(read).toHaveBeenCalledTimes(1);

  fireEvent.contextMenu(inside);
  fireEvent.click(item(/^Split right/));
  await waitFor(() => expect(useHive.getState().splits[0]).toMatchObject({ left: id }));
  fireEvent.contextMenu(inside);
  expect(item(/^Unsplit/)).toBeDefined();
  // Esc closes it without doing anything.
  fireEvent.keyDown(paneMenu(), { key: "Escape" });
  expect(screen.queryByRole("menu")).toBeNull();
  expect(clear).toHaveBeenCalledTimes(1);
  copy.mockRestore();
  read.mockRestore();
  write.mockRestore();
});

test("on macOS the terminal menu shows the Mac keys, ⌘K for Clear", async () => {
  asMac();
  const { inside } = await openShown();
  fireEvent.contextMenu(inside);
  expect(paneItems()).toEqual(["Copy ⌘C", "Paste ⌘V", "Clear ⌘K", "Split right ⇧⌘D"]);
});

/** Four terminals of fix-login, 1 and 2 a pair, 3 and 4 the pair shown with 4 focused (20.5). */
function twoPairs() {
  show();
  act(() => {
    for (const id of [1, 2, 3, 4]) addTab(id, fixLogin.path);
    setSplit({ left: 1, right: 2 });
    setSplit({ left: 3, right: 4 });
  });
  const boxes = () =>
    bar()
      .getAllByRole("tab")
      .map((t) => t.closest(".tab") as HTMLElement);
  return { boxes };
}

test("a pair's tabs are drawn as one group, the partner in view or just paired, described (20.5)", () => {
  const { boxes } = twoPairs();
  const attrs = (name: string) => boxes().map((b) => b.getAttribute(name));
  expect(attrs("data-pair")).toEqual(["left", "right", "left", "right"]);
  expect(attrs("data-split")).toEqual(["paired", "paired", "view", null]);
  expect(attrs("data-active")).toEqual(["false", "false", "false", "true"]);
  // The split icon on each group's first tab only.
  expect(boxes().map((b) => !!b.querySelector(".tab-split"))).toEqual([true, false, true, false]);
  const label = bar().getAllByRole("tab")[0] as HTMLElement;
  expect(label.getAttribute("aria-description")).toBe("Split with fix-login");
  expect(boxes()[0]?.getAttribute("title")).toBe(`${fixLogin.path}\nSplit with fix-login`);
  // Over a file, no pair is in view.
  act(() => setOpenFile({ worktree: fixLogin.id, path: "a.ts" }));
  expect(attrs("data-split").slice(0, 4)).toEqual(["paired", "paired", "paired", "paired"]);
  // Unpaired, a tab is plain again.
  act(() => unsplit(1));
  expect(boxes()[0]?.hasAttribute("data-pair")).toBe(false);
  expect(label.hasAttribute("aria-description")).toBe(false);
});

test("a paired tab's menu replaces its pane with another tab of the worktree, or swaps sides (20.5)", () => {
  const { boxes } = twoPairs();
  act(() => addTab(5, fixLogin.path));
  act(() => addTab(9, apiMain.path));
  act(() => select(fixLogin.id));
  const menu = () => screen.getByRole("menu", { name: "Terminal" });
  const items = () =>
    within(menu())
      .getAllByRole("menuitem")
      .map((i) => i.textContent);
  // A tab in no pair has neither item.
  fireEvent.contextMenu(bar().getAllByRole("tab")[4] as HTMLElement);
  expect(items()).toEqual(["Split right", "Close terminal"]);
  fireEvent.keyDown(menu(), { key: "Escape" });

  fireEvent.contextMenu(bar().getAllByRole("tab")[3] as HTMLElement);
  expect(items()).toEqual(["Unsplit", "Replace with", "Swap sides", "Close terminal"]);
  const replace = within(menu()).getByRole("menuitem", { name: "Replace with" });
  expect(replace.getAttribute("aria-expanded")).toBe("false");
  fireEvent.keyDown(replace, { key: "ArrowDown" });
  fireEvent.keyDown(replace, { key: "ArrowRight" });
  // Its submenu: the worktree's terminals in no other pane of this pair, the first focused.
  const sub = screen.getByRole("menu", { name: "Replace with" });
  const others = within(sub).getAllByRole("menuitem");
  expect(others.map((i) => i.textContent)).toEqual(["fix-login", "fix-login", "fix-login"]);
  expect(document.activeElement).toBe(others[0] as HTMLElement);
  fireEvent.click(replace);
  expect(screen.queryByRole("menu", { name: "Replace with" })).toBeNull();
  fireEvent.click(replace);
  fireEvent.click(
    within(screen.getByRole("menu", { name: "Replace with" })).getAllByRole(
      "menuitem",
    )[2] as HTMLElement,
  );
  expect(screen.queryByRole("menu")).toBeNull();
  expect(useHive.getState().splits).toEqual([
    { left: 1, right: 2 },
    { left: 3, right: 5 },
  ]);
  expect(useHive.getState().activeTab).toBe(5);

  fireEvent.contextMenu(bar().getAllByRole("tab")[0] as HTMLElement);
  fireEvent.click(within(menu()).getByRole("menuitem", { name: "Swap sides" }));
  expect(useHive.getState().splits).toEqual([
    { left: 3, right: 5 },
    { left: 2, right: 1 },
  ]);
  expect(boxes().map((b) => b.getAttribute("data-pair"))).toEqual([
    "left",
    "right",
    "left",
    "right",
    null,
  ]);

  // With no other terminal to take its place, Replace with is disabled.
  act(() => {
    for (const id of [3, 4, 5]) closeTerminal(id);
  });
  fireEvent.contextMenu(bar().getAllByRole("tab")[0] as HTMLElement);
  const none = within(menu()).getByRole("menuitem", { name: "Replace with" }) as HTMLButtonElement;
  expect(none.disabled).toBe(true);
});

test("the submenu opens on the menu's left when the window has no room on its right (20.5)", () => {
  twoPairs();
  const rect = spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    right: window.innerWidth + 10,
  } as DOMRect);
  fireEvent.contextMenu(bar().getAllByRole("tab")[0] as HTMLElement);
  fireEvent.click(screen.getByRole("menuitem", { name: "Replace with" }));
  expect(screen.getByRole("menu", { name: "Replace with" }).hasAttribute("data-flip")).toBe(true);
  rect.mockRestore();
});

test("a tab dragged onto the terminals highlights the pane it would take and takes it (20.5)", () => {
  const { boxes } = twoPairs();
  const host = document.querySelector(".terminal-host") as HTMLElement;
  spyOn(host, "getBoundingClientRect").mockReturnValue({ left: 0, width: 200 } as DOMRect);
  const data = { setData() {}, effectAllowed: "", dropEffect: "" };
  const drag = (type: "dragStart" | "dragOver" | "drop" | "dragLeave", el: HTMLElement, x = 0) => {
    const event = createEvent[type](el);
    Object.assign(event, { dataTransfer: data, clientX: x });
    fireEvent(el, event);
    return event;
  };
  const highlight = () => document.querySelector(".pane-drop")?.getAttribute("data-side") ?? null;
  // Nothing dragged: no highlight.
  drag("dragOver", host, 150);
  expect(highlight()).toBeNull();

  drag("dragStart", boxes()[0] as HTMLElement);
  // The split's divider is at 50%: x 150 is over the right pane.
  expect(drag("dragOver", host, 150).defaultPrevented).toBe(true);
  expect(highlight()).toBe("right");
  drag("dragOver", host, 50);
  expect(highlight()).toBe("left");
  drag("dragLeave", host);
  expect(highlight()).toBeNull();
  drag("dragOver", host, 150);
  drag("drop", host, 150);
  expect(highlight()).toBeNull();
  expect(useHive.getState().splits).toEqual([{ left: 3, right: 1 }]);
  fireEvent.dragEnd(boxes()[0] as HTMLElement);
  // A drop with nothing dragged does nothing.
  drag("drop", host, 150);
  expect(useHive.getState().splits).toEqual([{ left: 3, right: 1 }]);
});
