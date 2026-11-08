import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "../App";
import type { Script } from "../protocol";
import { apply } from "../reduce";
import { initialState, select, setPanelView, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [, login] = shop.worktrees;

const scripts: Script[] = [
  { name: "dev", command: "pnpm dev --port $PORT", source: "settings" },
  { name: "test", command: "pnpm run test", source: "package_json" },
  { name: "up", command: "task up", source: "taskfile" },
  { name: "build", command: "make build", source: "makefile" },
];

/** The app with shop, fix-login selected and the right panel on Scripts. */
function show() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => {
    select(login.id);
    setPanelView("scripts");
  });
}

const view = () => screen.getByRole("region", { name: "Scripts" });
const listed = (list: Script[], problems: string[] = [], port: number | null = 20_010) =>
  act(() => apply({ type: "scripts", path: login.path, scripts: list, problems, port }));

test("the tab asks for the worktree's scripts and lists them by source with their command", () => {
  const list = spyOn(transport, "listScripts").mockResolvedValue();
  show();
  expect(list).toHaveBeenCalledWith(login.path);
  expect(within(view()).getByText("Loading…")).toBeTruthy();
  listed(scripts, ["Makefile: 1 script left out (a name Hive cannot type as one word)"]);
  const groups = within(view()).getAllByRole("region");
  expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual([
    "Settings",
    "package.json",
    "Taskfile",
    "Makefile",
  ]);
  const rows = within(view()).getAllByRole("button", { name: /^/ });
  const runs = rows.filter((b) => b.classList.contains("script-main"));
  expect(runs.map((b) => [b.textContent, b.title])).toEqual(
    scripts.map((s) => [`${s.name}${s.command}`, `Run in a new terminal: ${s.command}`]),
  );
  expect(view().querySelector(".scripts-ports")?.textContent).toBe(
    "Ports 20010–20019$PORT 20010$HIVE_PORT 20010Preview",
  );
  expect(view().querySelector(".scripts-problem")?.textContent).toBe(
    "Makefile: 1 script left out (a name Hive cannot type as one word)",
  );
  // No block of ports: no strip.
  listed(scripts, [], null);
  expect(view().querySelector(".scripts-ports")).toBeNull();
  list.mockRestore();
});

test("a click runs a script in a new terminal; it shows as running, its terminal, and stops", async () => {
  const list = spyOn(transport, "listScripts").mockResolvedValue();
  const open = spyOn(transport, "openTerminal").mockResolvedValue(9);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  show();
  listed(scripts);
  fireEvent.click(within(view()).getByTitle("Run in a new terminal: task up"));
  await waitFor(() => expect(write).toHaveBeenCalledWith(9, "task up\r"));
  expect(open.mock.calls[0]?.[0]).toBe(login.path);
  const running = () => within(view()).queryByRole("region", { name: "Running" });
  await waitFor(() => expect(running()).not.toBeNull());
  expect(running()?.querySelector(".script")?.textContent).toBe("uptask up");
  // Another tab shown: the row's button shows the script's again.
  act(() => useHive.setState({ activeTab: null }));
  fireEvent.click(within(running() as HTMLElement).getByTitle("Show the terminal of up"));
  expect(useHive.getState().activeTab).toBe(9);
  fireEvent.click(within(running() as HTMLElement).getByTitle("Stop up (sends Ctrl+C)"));
  expect(write).toHaveBeenLastCalledWith(9, "\x03");
  expect(running()).toBeNull();
  list.mockRestore();
  open.mockRestore();
  write.mockRestore();
});

test("without scripts it points to Settings, opened on the project's scripts", () => {
  const list = spyOn(transport, "listScripts").mockResolvedValue();
  show();
  listed([]);
  expect(view().querySelector(".scripts-empty")?.textContent).toContain("No scripts.");
  fireEvent.click(within(view()).getByRole("button", { name: "Open Settings" }));
  expect([useHive.getState().modal, useHive.getState().modalProject]).toEqual([
    "settings",
    shop.id,
  ]);
  const dialog = screen.getByRole("dialog");
  // On the Projects section, this project picked.
  const picked = within(dialog).getByRole("combobox", { name: "Project" });
  expect(picked.textContent).toContain(shop.name);
  act(() => useHive.setState({ modal: null, modalProject: null }));
  // With scripts, the footer's link does the same.
  listed(scripts);
  fireEvent.click(
    within(view()).getByRole("button", { name: "Edit this project's scripts in Settings" }),
  );
  expect(useHive.getState().modalProject).toBe(shop.id);
  list.mockRestore();
});

test("Preview shows the dev server at a port of the block in a sandboxed frame, and back", () => {
  const list = spyOn(transport, "listScripts").mockResolvedValue();
  show();
  listed(scripts);
  const toggle = within(view()).getByRole("button", { name: "Preview" });
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  const frame = () => view().querySelector("iframe.page-frame");
  expect([frame()?.getAttribute("src"), frame()?.getAttribute("sandbox")]).toEqual([
    "http://localhost:20010/",
    "allow-scripts allow-same-origin allow-forms",
  ]);
  expect(frame()?.getAttribute("title")).toBe("Dev server at localhost:20010");
  // The scripts give way to the page.
  expect(within(view()).queryByRole("region", { name: "Settings" })).toBeNull();
  // Another port of the block, then a reload: a new frame at the same port.
  const port = within(view()).getByRole("combobox", { name: "Port" });
  expect(port.textContent).toContain("localhost:20010 ($PORT)");
  fireEvent.mouseDown(port);
  const ports = screen.getAllByRole("option").map((o) => o.textContent);
  expect([ports.length, ports.at(-1)]).toEqual([10, "localhost:20019"]);
  fireEvent.click(screen.getByRole("option", { name: "localhost:20013" }));
  expect(frame()?.getAttribute("src")).toBe("http://localhost:20013/");
  const before = frame();
  fireEvent.click(within(view()).getByRole("button", { name: "Reload the preview" }));
  expect(frame()).not.toBe(before);
  expect(frame()?.getAttribute("src")).toBe("http://localhost:20013/");
  fireEvent.click(toggle);
  expect(frame()).toBeNull();
  expect(within(view()).getByRole("region", { name: "Settings" })).toBeTruthy();
  list.mockRestore();
});
