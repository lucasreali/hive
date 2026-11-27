import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import type { HostLink, ServiceMessage, Space } from "../protocol";
import { apply } from "../reduce";
import { initialState, openModal, select, setPanelView, setRightPanel, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { CHOICES_LIMIT, useKept, viewKey } from "./panelPlace";
import { openSettings } from "./SettingsDialog";

// 28.27 (rows 11, 12, 13, 15, 16 of docs/review/last-place.md): choices kept for this run only
// as UI state: each worktree's Files search and Sessions search and Actions branch filter, the
// Settings section, the space switcher's opened hosts.

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const [, login, checkout] = shop.worktrees;

/** The app with shop and api, `worktree` selected and the right panel on `view`. */
function show(worktree: string, view: Parameters<typeof setPanelView>[0]) {
  spyOn(transport, "listSessions").mockResolvedValue();
  spyOn(transport, "listRuns").mockResolvedValue();
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop, api] });
    select(worktree);
    setPanelView(view);
  });
}
const searchbox = (name: string) => screen.getByRole<HTMLInputElement>("searchbox", { name });
const type = (name: string, value: string) =>
  fireEvent.change(searchbox(name), { target: { value } });
const pressed = (name: string) => screen.getByRole("button", { name }).getAttribute("aria-pressed");

/** A component showing choice `key` and setting it. */
function Choice({ name }: { name: string }) {
  const [value, set] = useKept(name, 0);
  return (
    <button type="button" onClick={() => set((was) => was + 1)}>
      {`${name}=${value}`}
    </button>
  );
}

test("a kept choice survives its component and is forgotten past the limit", () => {
  const { unmount } = render(<Choice name="a" />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.click(screen.getByRole("button"));
  unmount();
  render(<Choice name="a" />);
  expect(screen.getByRole("button").textContent).toBe("a=2");
  cleanup();
  // The least recently changed goes first.
  const many = Array.from({ length: CHOICES_LIMIT }, (_, i) => `k${i}`);
  for (const name of many) {
    render(<Choice name={name} />);
    fireEvent.click(screen.getByRole("button"));
    cleanup();
  }
  render(<Choice name="a" />);
  expect(screen.getByRole("button").textContent).toBe("a=0");
  cleanup();
  render(<Choice name="k1" />);
  expect(screen.getByRole("button").textContent).toBe("k1=1");
});

test("the Files search and its mode are each worktree's, kept while the view closes", () => {
  show(login.id, "files");
  type("Find files", "readme");
  fireEvent.click(screen.getByRole("button", { name: "Contents" }));
  // Another worktree: its own, empty search on Names.
  act(() => select(checkout.id));
  expect(searchbox("Find files").value).toBe("");
  expect(pressed("Names")).toBe("true");
  type("Find files", "src");
  // Back: the query and the mode it was left with.
  act(() => select(login.id));
  expect(searchbox("Find files").value).toBe("readme");
  expect(pressed("Contents")).toBe("true");
  // The view closed and shown again, and another view and back.
  act(() => setRightPanel(null));
  act(() => setRightPanel("files"));
  act(() => setPanelView("changes"));
  act(() => setPanelView("files"));
  expect(searchbox("Find files").value).toBe("readme");
  act(() => select(checkout.id));
  expect(searchbox("Find files").value).toBe("src");
});

test("the Sessions search is each worktree's, kept while the view closes", () => {
  show(login.id, "sessions");
  type("Search sessions", "login");
  act(() => setPanelView("files"));
  act(() => setPanelView("sessions"));
  expect(searchbox("Search sessions").value).toBe("login");
  act(() => select(checkout.id));
  expect(searchbox("Search sessions").value).toBe("");
  act(() => select(login.id));
  expect(searchbox("Search sessions").value).toBe("login");
});

test("the Actions branch filter is each worktree's, kept while the view closes", () => {
  show(login.id, "actions");
  const list = spyOn(transport, "listRuns").mockResolvedValue();
  const view = () => screen.getByRole("region", { name: "Actions" });
  const branch = () => within(view()).getByRole("combobox", { name: "Branch" }).textContent;
  fireEvent.mouseDown(within(view()).getByRole("combobox", { name: "Branch" }));
  fireEvent.click(screen.getByRole("option", { name: "All branches" }));
  expect(branch()).toBe("All branches");
  // Another worktree starts on its own branch.
  act(() => select(checkout.id));
  expect(branch()).toBe(checkout.branch);
  // Back, and after the view closed: All branches, asked again.
  act(() => select(login.id));
  expect(branch()).toBe("All branches");
  act(() => setPanelView("files"));
  list.mockClear();
  act(() => setPanelView("actions"));
  expect(branch()).toBe("All branches");
  expect(list).toHaveBeenCalledWith(shop.id, null, false);
});

test("Settings opens on the section picked last, unless one is asked for", () => {
  render(<App />);
  const current = () =>
    [...document.querySelectorAll(".settings-section")].find(
      (b) => b.getAttribute("aria-current") === "page",
    )?.textContent;
  act(() => openModal("settings"));
  expect(current()).toBe("Terminal");
  fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
  act(() => useHive.setState({ modal: null }));
  act(() => openModal("settings"));
  expect(current()).toBe("Appearance");
  // Asked for: Hosts (the hosts popover), Projects (a project's scripts).
  act(() => useHive.setState({ modal: null }));
  act(() => openSettings("Hosts"));
  expect(current()).toBe("Hosts");
  act(() => useHive.setState({ modal: null }));
  act(() => openModal("settings", shop.id));
  expect(current()).toBe("Projects");
  act(() => useHive.setState({ modal: null }));
  act(() => openModal("settings"));
  expect(current()).toBe("Appearance");
});

const NO_ENV = { git_name: null, git_email: null, gh_account: null, claude_account: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };

test("the space switcher's opened hosts stay open the next time it opens", () => {
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop, api] });
    apply({ type: "spaces", spaces: [home], current: "default" });
    for (const name of ["h1", "h2", "h3", "h4", "h5", "h6"]) {
      const link: HostLink = { state: "connected" };
      apply({ type: "host_link", host: name, ...link } as ServiceMessage);
      apply({ type: "spaces", spaces: [home], current: "default", host: name } as ServiceMessage);
    }
  });
  const picker = () => screen.getByRole("combobox", { name: "Space" });
  const list = () => screen.getByRole("listbox", { name: "Space" });
  fireEvent.mouseDown(picker());
  fireEvent.click(within(list()).getByRole("option", { name: "h2, 1 space, collapsed" }));
  fireEvent.keyDown(screen.getByRole("combobox", { name: "Filter spaces" }), { key: "Escape" });
  expect(screen.queryByRole("listbox")).toBeNull();
  // The sidebar hidden and shown again (its switcher made anew), then opened: h2 still open.
  act(() => useHive.setState({ sidebarShown: false }));
  act(() => useHive.setState({ sidebarShown: true }));
  fireEvent.mouseDown(picker());
  expect(within(list()).getByRole("option", { name: "h2, 1 space, open" })).toBeTruthy();
  expect(within(list()).getByRole("option", { name: "h1, 1 space, collapsed" })).toBeTruthy();
});

test("a view's choices are keyed by host too", () => {
  expect(viewKey("files-query", "gpu", "/w")).not.toBe(viewKey("files-query", null, "/w"));
});
