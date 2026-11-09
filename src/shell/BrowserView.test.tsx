import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "../App";
import { apply } from "../reduce";
import { closeActiveTab, cycleTab, showBarItem } from "../shortcuts";
import {
  activateTab,
  addTab,
  closeBrowserTab,
  initialState,
  removeTab,
  select,
  setOpenFile,
  setSplit,
  setVisible,
  showBesideAgent,
  showBrowserTab,
  useHive,
} from "../store";
import { barItems, browserKey, fileKey, shownBrowser, shownPane, shownPanes } from "../tabs";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";
import { paletteCommands } from "./Palette";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [main, login] = shop.worktrees;
const s = () => useHive.getState();

/** The app with shop and fix-login selected. */
function show() {
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop] });
    select(login.id);
  });
}

const view = () => screen.getByRole("region", { name: "Agent's browser" });
const bar = () => screen.getByRole("tablist", { name: "Open terminals and files" });
const status = (
  worktree: string,
  running: boolean,
  url: string | null = null,
  error: string | null = null,
  title: string | null = null,
) => act(() => apply({ type: "browser_status", worktree, running, url, title, error }));
const action = (
  text: string,
  worktree = login.id,
  agent = "s1",
  agent_title: string | null = null,
) => act(() => apply({ type: "browser_action", worktree, agent, agent_title, action: text }));
const items = () =>
  barItems(s()).map((t) => ("path" in t ? t.path : "browser" in t ? "browser" : t.id));

test("the first call opens the tab at the bar's end, unfocused; closed, it stays closed while that browser runs", () => {
  show();
  act(() => addTab(1, login.path));
  status(login.id, true);
  action("clicked 'A'");
  expect(items()).toEqual([1, "browser"]);
  // Without the focus.
  expect(s().activeTab).toBe(1);
  expect(shownBrowser(s())).toBeNull();
  expect(screen.queryByRole("region", { name: "Agent's browser" })).toBeNull();
  // One per worktree.
  action("clicked 'B'");
  expect(items()).toEqual([1, "browser"]);
  // Closed: the next calls do not bring it back, though the caption goes on.
  fireEvent.click(within(bar()).getByRole("button", { name: "Close browser Browser" }));
  expect(items()).toEqual([1]);
  action("clicked 'C'");
  expect(items()).toEqual([1]);
  expect(s().browserActions[login.id]?.action).toBe("clicked 'C'");
  // That browser stopped: the next one's first call opens it again.
  status(login.id, false);
  status(login.id, true);
  action("clicked 'D'");
  expect(items()).toEqual([1, "browser"]);
});

test("a tab closed once its browser stopped comes back with the next browser", () => {
  show();
  status(login.id, true);
  action("clicked 'A'");
  status(login.id, false);
  act(() => closeBrowserTab(login.id));
  expect(s().browserClosed).toEqual({});
  status(login.id, true);
  action("clicked 'B'");
  expect(items()).toEqual(["browser"]);
});

test("the tab: a globe, the page's title else its URL, its URL as the tooltip", () => {
  show();
  action("clicked 'A'");
  const tab = () => within(bar()).getByRole("tab");
  expect(tab().textContent).toBe("Browser");
  expect(tab().closest(".tab")?.getAttribute("title")).toBe("Browser");
  expect(tab().querySelector("svg")).not.toBeNull();
  status(login.id, true, "https://a.test/");
  expect(tab().textContent).toBe("https://a.test/");
  status(login.id, true, "https://a.test/", null, "Shop");
  expect(tab().textContent).toBe("Shop");
  expect(tab().closest(".tab")?.getAttribute("title")).toBe("https://a.test/");
  // Clicking it shows it, in front of the terminals; a terminal's click shows that again.
  fireEvent.click(tab());
  expect(tab().getAttribute("aria-selected")).toBe("true");
  expect(view()).toBeDefined();
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(true);
});

test("the screencast runs only while the tab is shown and the window visible", () => {
  const watch = spyOn(transport, "watchBrowser").mockResolvedValue();
  show();
  action("clicked 'A'");
  expect(watch).not.toHaveBeenCalled();
  act(() => showBrowserTab(login.id));
  expect(watch.mock.calls).toEqual([[login.id]]);
  act(() => setVisible(false));
  expect(watch.mock.calls.at(-1)).toEqual([null]);
  act(() => setVisible(true));
  expect(watch.mock.calls.at(-1)).toEqual([login.id]);
  // Another tab of the bar: none.
  act(() => addTab(1, login.path));
  expect(watch.mock.calls.at(-1)).toEqual([null]);
  expect(shownBrowser(s())).toBeNull();
  watch.mockRestore();
});

test("in the tab: the URL, the caption naming its agent, each frame acknowledged once drawn", async () => {
  const watch = spyOn(transport, "watchBrowser").mockResolvedValue();
  const drawn = spyOn(transport, "browserDrawn").mockResolvedValue();
  show();
  act(() => showBrowserTab(login.id));
  status(login.id, true, "http://localhost:5173/");
  expect(view().querySelector(".browser-url")?.textContent).toBe("http://localhost:5173/");
  expect(view().querySelector(".browser-caption")).toBeNull();
  expect(screen.queryByRole("img")).toBeNull();
  action("clicked 'Learn more'");
  expect(view().querySelector(".browser-caption")?.textContent).toBe(
    "Claude · clicked 'Learn more'",
  );
  // Plain text: markup in a caption stays text.
  action("typed '<b>x</b>'", login.id, "s2", "Fix login");
  const caption = view().querySelector(".browser-caption") as HTMLElement;
  expect(caption.textContent).toBe("Fix login · typed '<b>x</b>'");
  expect(caption.querySelector("b")).toBeNull();
  // Another worktree's frame is not this one's.
  act(() => apply({ type: "browser_frame", worktree: main.id, data: "BBBB" }));
  expect(screen.queryByRole("img")).toBeNull();
  act(() => apply({ type: "browser_frame", worktree: login.id, data: "AAAA" }));
  const img = screen.getByRole("img", { name: "The agent's browser" }) as HTMLImageElement;
  expect(img.src).toBe("data:image/jpeg;base64,AAAA");
  await waitFor(() => expect(drawn).toHaveBeenCalledTimes(1));
  // The same picture again is a new frame, acknowledged too.
  act(() => apply({ type: "browser_frame", worktree: login.id, data: "AAAA" }));
  await waitFor(() => expect(drawn).toHaveBeenCalledTimes(2));
  status(login.id, true);
  expect(view().querySelector(".browser-url")?.textContent).toBe("No page");
  watch.mockRestore();
  drawn.mockRestore();
});

test('"Open in my browser" offers only a web page', () => {
  show();
  act(() => showBrowserTab(login.id));
  const open = () =>
    screen.getByRole("button", { name: "Open in my browser" }) as HTMLButtonElement;
  expect(open().disabled).toBe(true);
  for (const blocked of ["about:blank", "file:///etc/passwd", "javascript:alert(1)"]) {
    status(login.id, true, blocked);
    expect(open().disabled).toBe(true);
  }
  status(login.id, true, "https://a.test/x");
  expect(open().disabled).toBe(false);
  // Outside the app nothing opens, and it says so with the URL only.
  fireEvent.click(open());
  expect(JSON.stringify(s().notices ?? s())).toContain("https://a.test/x");
});

test("stopped, the tab stays with its frame, URL and caption; a new browser takes the same tab", () => {
  spyOn(transport, "browserDrawn").mockResolvedValue();
  show();
  act(() => showBrowserTab(login.id));
  status(login.id, true, "https://a.test/");
  action("clicked 'A'");
  act(() => apply({ type: "browser_frame", worktree: login.id, data: "AAAA" }));
  expect(view().textContent).not.toContain("Browser closed");
  status(login.id, false, "https://a.test/");
  expect(view().textContent).toContain("Browser closed");
  expect(view().textContent).toContain("https://a.test/");
  expect(view().textContent).toContain("clicked 'A'");
  expect(screen.getByRole("img", { name: "The agent's browser" })).toBeDefined();
  // Why it did not start, when it did not.
  status(login.id, false, "https://a.test/", "No more browsers");
  expect(view().textContent).toContain("No more browsers");
  // A new browser: the same tab, no old picture.
  status(login.id, true);
  expect(items()).toEqual(["browser"]);
  expect(view().textContent).not.toContain("Browser closed");
  expect(screen.queryByRole("img")).toBeNull();
  // Still running: a frame stays through a new status.
  act(() => apply({ type: "browser_frame", worktree: login.id, data: "AAAA" }));
  status(login.id, true, "https://b.test/");
  expect(screen.getByRole("img", { name: "The agent's browser" })).toBeDefined();
  // Another worktree's new browser leaves this one's picture.
  status(main.id, true);
  expect(screen.getByRole("img", { name: "The agent's browser" })).toBeDefined();
  // The service gone: closed.
  act(() => apply({ type: "disconnected", reason: "gone", bundled: true }));
  expect(s().browsers).toEqual({});
  expect(view().textContent).toContain("Browser closed");
});

test("the agent's row shows a globe while it browses, pulsing at each action; a click shows the tab", () => {
  show();
  act(() => {
    apply({
      type: "agent_detected",
      channel: 1,
      id: "s1",
      project: shop.id,
      worktree: login.id,
      cwd: login.path,
    });
    apply({
      type: "agent_detected",
      channel: 2,
      id: "s2",
      project: null,
      worktree: null,
      cwd: "/x",
    });
    apply({ type: "agent_state", id: "s1", ...agentStatus("working"), subagents: [] });
  });
  const globe = () => screen.queryByRole("button", { name: "Show the agent's browser" });
  expect(globe()).toBeNull();
  const browsing = (id: string, on: boolean) =>
    act(() =>
      apply({ type: "agent_state", id, ...agentStatus("working"), browsing: on, subagents: [] }),
    );
  // An agent in no worktree has no browser to show.
  browsing("s2", true);
  expect(globe()).toBeNull();
  browsing("s1", true);
  const first = globe()?.querySelector("svg");
  expect(first).toBeDefined();
  // A new element at each action, which plays the pulse once.
  action("clicked 'A'");
  const second = globe()?.querySelector("svg");
  expect(second).not.toBe(first);
  // Closed by the human, the globe brings it back and shows it.
  act(() => closeBrowserTab(login.id));
  act(() => select(main.id));
  fireEvent.click(globe() as HTMLElement);
  expect(items()).toEqual(["browser"]);
  expect(s().selection).toBe(login.id);
  expect(shownBrowser(s())).toBe(login.id);
  expect(view()).toBeDefined();
  browsing("s1", false);
  expect(globe()).toBeNull();
});

test("the palette shows the running browser's tab", () => {
  show();
  const command = () => paletteCommands(s()).find((c) => c.label === "Show the agent's browser");
  expect(command()).toBeUndefined();
  status(login.id, true);
  expect(command()?.detail).toBe(login.name);
  act(() => command()?.run());
  expect(shownBrowser(s())).toBe(login.id);
  status(login.id, false);
  expect(command()).toBeUndefined();
});

test("it takes its turn with the bar's other tabs: the keys, closing, and moving between worktrees", () => {
  show();
  act(() => {
    addTab(1, login.path);
    setOpenFile({ worktree: login.id, path: "a.ts" });
    addTab(2, login.path);
  });
  status(login.id, true);
  action("clicked 'A'");
  expect(items()).toEqual([1, "a.ts", 2, "browser"]);
  // The next key from the last terminal reaches it, and from it the first.
  act(() => cycleTab(1));
  expect(shownBrowser(s())).toBe(login.id);
  act(() => cycleTab(-1));
  expect([shownBrowser(s()), s().activeTab]).toEqual([null, 2]);
  act(() => showBarItem(3));
  expect(shownBrowser(s())).toBe(login.id);
  // A file shown hides it.
  act(() => setOpenFile({ worktree: login.id, path: "a.ts" }));
  expect(shownBrowser(s())).toBeNull();
  act(() => showBarItem(3));
  // A terminal closing while it shows leaves it shown.
  act(() => removeTab(1));
  expect(shownBrowser(s())).toBe(login.id);
  // ⌘W closes it: the tab beside it shows.
  act(() => closeActiveTab());
  expect(items()).toEqual(["a.ts", 2]);
  expect([shownBrowser(s()), s().activeTab, s().fileShown]).toEqual([null, 2, false]);
  expect(s().browserClosed).toEqual({ [login.id]: true });
  // Closed while hidden, what shows stays.
  act(() => showBrowserTab(login.id));
  act(() => setOpenFile({ worktree: login.id, path: "a.ts" }));
  act(() => closeBrowserTab(login.id));
  expect(s().fileShown).toBe(true);
});

test("closing what shows beside it shows it; a place with only it shows it", () => {
  show();
  act(() => addTab(1, login.path));
  action("clicked 'A'");
  // The terminal goes: the tab beside it, the browser's, shows.
  act(() => removeTab(1));
  expect(shownBrowser(s())).toBe(login.id);
  // Another worktree hides it; back there, it is all there is, so it shows.
  act(() => select(main.id));
  expect(shownBrowser(s())).toBeNull();
  expect(items()).toEqual([]);
  act(() => select(login.id));
  expect(shownBrowser(s())).toBe(login.id);
  // Closing the only tab shows nothing.
  act(() => closeBrowserTab(login.id));
  expect(shownBrowser(s())).toBeNull();
  // A file closing shows the browser beside it.
  action("clicked 'B'", login.id);
  act(() => setOpenFile({ worktree: login.id, path: "a.ts" }));
  act(() => setOpenFile(null));
  expect(items()).toEqual(["browser"]);
  expect(shownBrowser(s())).toBe(login.id);
  // Selecting its place again keeps it in front of a file there.
  act(() => setOpenFile({ worktree: login.id, path: "a.ts" }));
  act(() => showBrowserTab(login.id));
  act(() => select(login.id));
  expect([shownBrowser(s()), s().fileShown]).toEqual([login.id, false]);
  // Coming back to a place with a file and it, the file shows, as in a place without it.
  act(() => select(main.id));
  act(() => select(login.id));
  expect([shownBrowser(s()), s().fileShown]).toEqual([null, true]);
});

test("with nothing selected every worktree's tab is in the bar", () => {
  show();
  action("clicked 'A'");
  action("clicked 'B'", main.id, "s2");
  act(() => select(null));
  expect(items()).toEqual(["browser", "browser"]);
  act(() => showBrowserTab(main.id));
  expect(shownBrowser(s())).toBe(main.id);
});

test("the tab goes with its worktree, and with its project", () => {
  show();
  action("clicked 'A'");
  action("clicked 'B'", main.id, "s2");
  act(() => showBrowserTab(login.id));
  // Its worktree removed.
  const without = { ...shop, worktrees: [main] };
  act(() => apply({ type: "projects", projects: [without] }));
  expect(s().browserTabs).toEqual([{ browser: main.id }]);
  expect(s().browserShown).toBeNull();
  expect(s().tabOrder).toEqual([`browser:${main.id}`]);
  // Another worktree's stays shown through a new list.
  act(() => showBrowserTab(main.id));
  act(() => apply({ type: "projects", projects: [without] }));
  expect(s().browserShown).toBe(main.id);
  // Its project removed.
  act(() => apply({ type: "project_removed", id: shop.id }));
  expect(s().browserTabs).toEqual([]);
  expect(s().browserShown).toBeNull();
  expect(s().tabOrder).toEqual([]);
});

test("another project's tab stays when one project goes", () => {
  show();
  const [, api] = MOCK_REPOS;
  act(() => apply({ type: "projects", projects: [shop, api] }));
  action("clicked 'A'", api.worktrees[0]?.id);
  act(() => showBrowserTab(api.worktrees[0]?.id as string));
  act(() => apply({ type: "project_removed", id: shop.id }));
  expect(s().browserTabs).toHaveLength(1);
  expect(s().browserShown).toBe(api.worktrees[0]?.id as string);
});

test('"Show beside the agent" pairs the tab with its last action\'s agent, only when asked', () => {
  show();
  act(() => {
    addTab(1, login.path);
    addTab(2, login.path);
    apply({
      type: "agent_detected",
      channel: 1,
      id: "s1",
      project: shop.id,
      worktree: login.id,
      cwd: login.path,
    });
    apply({
      type: "agent_detected",
      channel: 2,
      id: "s2",
      project: shop.id,
      worktree: login.id,
      cwd: login.path,
    });
  });
  const key = browserKey(login.id);
  status(login.id, true);
  // A browser tool call never pairs by itself.
  action("clicked 'A'");
  expect(s().splits).toEqual([]);
  action("clicked 'B'", login.id, "s2");
  const menu = () => fireEvent.contextMenu(within(bar()).getByRole("tab", { name: "Browser" }));
  const item = (name: string) => screen.getByRole("menuitem", { name }) as HTMLButtonElement;
  menu();
  expect(screen.queryByRole("menuitem", { name: "Unsplit" })).toBeNull();
  expect(screen.queryByRole("menuitem", { name: "Swap sides" })).toBeNull();
  fireEvent.click(item("Show beside the agent"));
  // The agent of the last action, the second one; both show, the browser focused.
  expect(s().splits).toEqual([{ left: 2, right: key }]);
  expect(shownPanes(s())).toEqual([2, key]);
  expect(shownPane(s())).toBe(key);
  expect(view().getAttribute("data-pane")).toBe("right");
  // Its screencast still needs the tab in view: the pair shown from the terminal keeps it.
  act(() => activateTab({ id: 2, cwd: login.path }));
  expect(shownPanes(s())).toEqual([2, key]);
  expect(view()).toBeDefined();
  // A click in the page focuses its pane.
  fireEvent.pointerDown(view());
  expect(shownPane(s())).toBe(key);
  // Already beside it: nothing more to do; the partner's name is on the tab.
  menu();
  expect(item("Show beside the agent").disabled).toBe(true);
  fireEvent.click(item("Swap sides"));
  expect(s().splits).toEqual([{ left: key, right: 2 }]);
  menu();
  fireEvent.click(item("Unsplit"));
  expect(s().splits).toEqual([]);
  // The agent gone: no terminal to pair with.
  act(() => apply({ type: "agent_removed", channel: 2, id: "s2" }));
  menu();
  expect(item("Show beside the agent").disabled).toBe(true);
  fireEvent.click(item("Close browser"));
  expect(s().browserTabs).toEqual([]);
  // Asked with no terminal, nothing happens.
  act(() => showBesideAgent(login.id));
  expect(s().splits).toEqual([]);
});

test("in a pair, closing either side leaves the other shown", () => {
  show();
  act(() => {
    addTab(1, login.path);
    addTab(2, login.path);
  });
  const key = browserKey(login.id);
  action("clicked 'A'");
  const pair = () => act(() => setSplit({ left: 1, right: key }));
  // The browser's tab closes: the terminal beside it shows, not the tab next to it in the bar.
  pair();
  act(() => closeBrowserTab(login.id));
  expect(s().splits).toEqual([]);
  expect([shownPanes(s()), s().activeTab]).toEqual([[1], 1]);
  // The terminal closes while it has the focus: the browser beside it shows.
  action("clicked 'B'");
  act(() => showBrowserTab(login.id));
  status(login.id, false);
  action("clicked 'C'");
  pair();
  act(() => activateTab({ id: 1, cwd: login.path }));
  act(() => removeTab(1));
  expect(s().splits).toEqual([]);
  expect(shownBrowser(s())).toBe(login.id);
  // A file paired with it closing shows it too.
  act(() => setOpenFile({ worktree: login.id, path: "a.ts" }));
  act(() =>
    setSplit(
      { left: fileKey({ worktree: login.id, path: "a.ts" }), right: key },
      fileKey({ worktree: login.id, path: "a.ts" }),
    ),
  );
  act(() => setOpenFile(null));
  expect(shownBrowser(s())).toBe(login.id);
  // Focus on a tab that is neither: a pair of a terminal and a file key leaves it alone.
  act(() => setSplit({ left: 2, right: key }, 2));
  expect([shownPane(s()), shownBrowser(s())]).toEqual([2, null]);
  act(() => setSplit({ left: 2, right: key }, "unknown"));
  expect(shownPane(s())).toBe(2);
});

test("a pair goes with its browser's tab, when its worktree or its project does", () => {
  show();
  action("clicked 'A'", main.id, "s2");
  action("clicked 'B'");
  act(() => {
    addTab(1, login.path);
    addTab(2, main.path);
    setSplit({ left: 1, right: browserKey(login.id) });
  });
  act(() => select(main.id));
  act(() => setSplit({ left: 2, right: browserKey(main.id) }));
  // The list unchanged: the pairs stay.
  act(() => apply({ type: "projects", projects: [shop] }));
  expect(s().splits).toHaveLength(2);
  act(() => apply({ type: "projects", projects: [{ ...shop, worktrees: [main] }] }));
  expect(s().splits).toEqual([{ left: 2, right: browserKey(main.id) }]);
  act(() => apply({ type: "project_removed", id: shop.id }));
  expect(s().splits).toEqual([]);
});
