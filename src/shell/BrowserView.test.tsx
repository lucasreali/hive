import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { apply } from "../reduce";
import { initialState, select, setPanelView, setVisible, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [main, login] = shop.worktrees;

/** The app with shop, fix-login selected and the right panel on Browser (its setting on). */
function show() {
  render(<App />);
  act(() => {
    const settings = useHive.getState().settings;
    apply({
      type: "settings",
      settings: { ...settings, browser: { ...settings.browser, enabled: true } },
    });
    apply({ type: "projects", projects: [shop] });
    select(login.id);
    setPanelView("browser");
  });
}

const view = () => screen.getByRole("region", { name: "Browser" });
const status = (worktree: string, running: boolean, url: string | null, error: string | null) =>
  act(() => apply({ type: "browser_status", worktree, running, url, error }));

test("the tab watches its worktree's browser while shown and the window visible", () => {
  const watch = spyOn(transport, "watchBrowser").mockResolvedValue();
  show();
  expect(watch.mock.calls).toEqual([[login.id]]);
  act(() => setVisible(false));
  expect(watch.mock.calls.at(-1)).toEqual([null]);
  act(() => setVisible(true));
  expect(watch.mock.calls.at(-1)).toEqual([login.id]);
  // Another worktree: that one's; another tab: none.
  act(() => select(main.id));
  expect(watch.mock.calls.slice(-2)).toEqual([[null], [main.id]]);
  act(() => setPanelView("files"));
  expect(watch.mock.calls.at(-1)).toEqual([null]);
  watch.mockRestore();
});

test("it shows why there is no browser, else the page and each frame, acknowledged once drawn", async () => {
  const watch = spyOn(transport, "watchBrowser").mockResolvedValue();
  const drawn = spyOn(transport, "browserDrawn").mockResolvedValue();
  show();
  expect(view().textContent).toContain("No browser yet");
  status(login.id, false, null, "No Chrome, Chromium, Edge or Brave found");
  expect(view().textContent).toContain("No Chrome, Chromium, Edge or Brave found");
  status(login.id, true, "http://localhost:5173/", null);
  expect(view().querySelector(".browser-url")?.textContent).toBe("http://localhost:5173/");
  expect(screen.queryByRole("img")).toBeNull();
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
  status(login.id, true, null, null);
  expect(view().querySelector(".browser-url")?.textContent).toBe("No page");
  // Stopped: no picture.
  status(login.id, false, null, null);
  expect(screen.queryByRole("img")).toBeNull();
  watch.mockRestore();
  drawn.mockRestore();
});
