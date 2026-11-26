import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import type { HostLink, ServiceMessage, Space } from "../protocol";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { type HostSpaces, hostSpaces, switcherSections } from "./SpaceSwitcher";

// 27.8: the space switcher grouped by host, this Mac first; with many hosts a filter, the shown
// host first and the others collapsed; one "New space…" whose dialog asks where.

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const NO_ENV = { git_name: null, git_email: null, gh_account: null, claude_account: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };
const work: Space = { id: "w", name: "Work", projects: [api.id], env: NO_ENV };

/** The app with this Mac's two spaces, Home shown. */
function show() {
  render(<App />);
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    apply({ type: "projects", projects: [shop, api] });
    apply({ type: "spaces", spaces: [home, work], current: "default" });
  });
}

/** Host `host` linked in `state`, with `spaces` (Home and Work by default). */
function host(name: string, state: HostLink = { state: "connected" }, spaces = [home, work]) {
  act(() => {
    apply({ type: "host_link", host: name, ...state } as ServiceMessage);
    apply({ type: "spaces", spaces, current: spaces[0].id, host: name } as ServiceMessage);
  });
}

const picker = () => screen.getByRole("combobox", { name: "Space" });
const list = () => screen.getByRole("listbox", { name: "Space" });
/** Each option's accessible name, and whether it is disabled. */
const options = () =>
  within(list())
    .getAllByRole("option")
    .map((o) => {
      const name = o.getAttribute("aria-label") ?? o.textContent;
      return o.getAttribute("aria-disabled") ? `(${name})` : name;
    });
/** The sections' headers, each with its dot's state. */
const headers = () =>
  [...document.querySelectorAll(".space-host")].map(
    (h) => `${h.querySelector(".connection")?.getAttribute("data-status")} ${h.textContent}`,
  );
const open = () => fireEvent.mouseDown(picker());
const option = (name: string) => within(list()).getByRole("option", { name });

test("one section per host, this Mac first, with its link; one New space… (27.8)", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  show();
  // A host's data before its link is known: not listed.
  act(() => apply({ type: "spaces", spaces: [home], current: "default", host: "early" }));
  host("gpu-box");
  host("lab", { state: "reconnecting", reason: "gone", retry_secs: 5 }, [home]);
  open();
  expect(headers()).toEqual([
    "connected This Mac",
    "connected gpu-box",
    "reconnecting labreconnecting",
  ]);
  // A down host's spaces dimmed, not to be picked.
  expect(options()).toEqual([
    "Home",
    "Work",
    "Home · gpu-box",
    "Work · gpu-box",
    "(Home · lab)",
    "New space…",
    "Edit space…",
  ]);
  expect(option("Home · lab").hasAttribute("data-dimmed")).toBe(true);
  fireEvent.click(option("Home · lab"));
  expect(useHive.getState().host).toBeNull();
  // The current space is checked; the trigger names a remote's host.
  expect(option("Home").getAttribute("aria-selected")).toBe("true");
  fireEvent.click(option("Work · gpu-box"));
  expect(useHive.getState().host).toBe("gpu-box");
  expect(select).toHaveBeenCalledWith("w");
  expect(picker().textContent).toBe("Home · gpu-box");
  expect(screen.queryByRole("listbox")).toBeNull();
  // A host without spaces says why.
  act(() => apply({ type: "host_link", host: "new", state: "connecting" } as ServiceMessage));
  open();
  expect(document.querySelector(".space-section:last-of-type .space-hint")?.textContent).toBe(
    "connecting",
  );
  // "Edit space…" and "New space…" open their dialogs, on the host shown.
  fireEvent.click(option("Edit space…"));
  expect(useHive.getState().modal).toBe("edit-space");
  act(() => useHive.setState({ modal: null }));
  open();
  fireEvent.click(option("New space…"));
  expect(useHive.getState().modal).toBe("new-space");
  expect(useHive.getState().host).toBe("gpu-box");
  select.mockRestore();
});

test("a host's space shows its projects; while its link is down it stays, dimmed (26.18)", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  show();
  act(() => {
    apply({ type: "host_link", host: "gpu-box", state: "connected" } as ServiceMessage);
    apply({
      type: "spaces",
      spaces: [home, work],
      current: "w",
      host: "gpu-box",
    } as ServiceMessage);
    apply({ type: "projects", projects: [api], host: "gpu-box" } as ServiceMessage);
  });
  const projectRows = () =>
    [...document.querySelectorAll(".tree-row.project .label")].map((r) => r.textContent);
  expect(projectRows()).toEqual(["shop"]);
  // Its current space shows at once, with its projects; no request.
  open();
  fireEvent.click(option("Work · gpu-box"));
  expect(picker().textContent).toBe("Work · gpu-box");
  expect(projectRows()).toEqual(["api"]);
  expect(select).not.toHaveBeenCalled();
  // Its link down: the space stays shown, its tree dimmed.
  act(() =>
    apply({
      type: "host_link",
      host: "gpu-box",
      state: "reconnecting",
      reason: "x",
      retry_secs: 1,
    } as ServiceMessage),
  );
  expect(picker().textContent).toBe("Work · gpu-box");
  expect(document.querySelector(".sidebar-tree")?.hasAttribute("data-dimmed")).toBe(true);
  select.mockRestore();
});

test("the keyboard opens, moves through and picks; Esc, Tab and a click outside close", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  show();
  host("gpu-box");
  const key = (k: string) => fireEvent.keyDown(picker(), { key: k });
  key("x");
  expect(screen.queryByRole("listbox")).toBeNull();
  key("ArrowDown");
  const active = () =>
    document
      .getElementById(picker().getAttribute("aria-activedescendant") ?? "")
      ?.getAttribute("aria-label") ??
    document.getElementById(picker().getAttribute("aria-activedescendant") ?? "")?.textContent;
  expect(active()).toBe("Home");
  key("ArrowDown");
  key("ArrowDown");
  expect(active()).toBe("Home · gpu-box");
  key("End");
  expect(active()).toBe("Edit space…");
  key("ArrowDown");
  expect(active()).toBe("Edit space…");
  key("Home");
  key("ArrowUp");
  expect(active()).toBe("Home");
  key("x");
  key("ArrowDown");
  key("ArrowDown");
  key(" ");
  // Its current space: shown at once, no request.
  expect(select).not.toHaveBeenCalled();
  expect(useHive.getState().host).toBe("gpu-box");
  key("Enter");
  expect(screen.getByRole("listbox")).toBeDefined();
  key("Escape");
  expect(screen.queryByRole("listbox")).toBeNull();
  key(" ");
  key("Tab");
  expect(screen.queryByRole("listbox")).toBeNull();
  // A second press on the trigger, a click outside or a resize closes it.
  open();
  open();
  expect(screen.queryByRole("listbox")).toBeNull();
  open();
  fireEvent.mouseMove(option("Work"));
  expect(option("Work").getAttribute("data-active")).toBe("true");
  // A press inside keeps the focus where it is.
  const inside = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  option("Work").dispatchEvent(inside);
  expect(inside.defaultPrevented).toBe(true);
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("listbox")).toBeNull();
  open();
  fireEvent(window, new Event("resize"));
  expect(screen.queryByRole("listbox")).toBeNull();
  // Only a left press opens it.
  fireEvent.mouseDown(picker(), { button: 2 });
  expect(screen.queryByRole("listbox")).toBeNull();
  select.mockRestore();
});

test("many hosts: a filter, the shown host first and the others collapsed until opened", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  show();
  for (const name of ["h1", "h2", "h3", "h4", "h5", "h6"]) host(name);
  open();
  const filter = screen.getByRole("combobox", { name: "Filter spaces" });
  expect(document.activeElement).toBe(filter);
  expect(options()).toEqual([
    "Home",
    "Work",
    "h1, 2 spaces, collapsed",
    "h2, 2 spaces, collapsed",
    "h3, 2 spaces, collapsed",
    "h4, 2 spaces, collapsed",
    "h5, 2 spaces, collapsed",
    "h6, 2 spaces, collapsed",
    "New space…",
    "Edit space…",
  ]);
  // A header opens on a click and closes on another.
  fireEvent.click(option("h2, 2 spaces, collapsed"));
  expect(options().slice(3, 6)).toEqual(["h2, 2 spaces, open", "Home · h2", "Work · h2"]);
  fireEvent.click(option("h2, 2 spaces, open"));
  expect(options()[3]).toBe("h2, 2 spaces, collapsed");
  // And on → / ← from the filter.
  const key = (k: string) => fireEvent.keyDown(filter, { key: k });
  key("ArrowDown");
  key("ArrowDown");
  const active = () =>
    document.getElementById(filter.getAttribute("aria-activedescendant") ?? "")?.ariaLabel ??
    document
      .getElementById(filter.getAttribute("aria-activedescendant") ?? "")
      ?.getAttribute("aria-label");
  expect(active()).toBe("h1, 2 spaces, collapsed");
  key("ArrowLeft");
  expect(active()).toBe("h1, 2 spaces, collapsed");
  key("ArrowRight");
  expect(active()).toBe("h1, 2 spaces, open");
  key("ArrowRight");
  expect(active()).toBe("h1, 2 spaces, open");
  key("ArrowLeft");
  expect(active()).toBe("h1, 2 spaces, collapsed");
  // In the filter, Home, End and Space are the field's.
  key("End");
  key(" ");
  expect(active()).toBe("h1, 2 spaces, collapsed");
  key("Enter");
  expect(active()).toBe("h1, 2 spaces, open");
  // A host's name matches its section, whole; a space's name its own row.
  fireEvent.change(filter, { target: { value: "h5" } });
  expect(options()).toEqual(["Home · h5", "Work · h5", "New space…", "Edit space…"]);
  fireEvent.change(filter, { target: { value: "wor" } });
  expect(options()).toEqual([
    "Work",
    "Work · h1",
    "Work · h2",
    "Work · h3",
    "Work · h4",
    "Work · h5",
    "Work · h6",
    "New space…",
    "Edit space…",
  ]);
  fireEvent.change(filter, { target: { value: "this mac" } });
  expect(options()).toEqual(["Home", "Work", "New space…", "Edit space…"]);
  fireEvent.change(filter, { target: { value: "nothing" } });
  expect(screen.getByText("No space or host matches")).toBeDefined();
  // A press in the filter keeps it; picking with Enter.
  const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  filter.dispatchEvent(press);
  expect(press.defaultPrevented).toBe(false);
  fireEvent.change(filter, { target: { value: "h6" } });
  key("ArrowDown");
  key("Enter");
  expect(useHive.getState().host).toBe("h6");
  expect(select).toHaveBeenCalledWith("w");
  // Shown, its section comes first and open; the filter starts empty again.
  open();
  const shown = screen.getByRole("combobox", { name: "Filter spaces" }) as HTMLInputElement;
  expect(shown.value).toBe("");
  expect(options().slice(0, 3)).toEqual([
    "Home · h6",
    "Work · h6",
    "This Mac, 2 spaces, collapsed",
  ]);
  // Esc in the filter closes the list, not the filter only.
  fireEvent.keyDown(shown, { key: "Escape" });
  expect(screen.queryByRole("listbox")).toBeNull();
  expect(document.activeElement).toBe(picker());
  select.mockRestore();
});

test("the sections, as the switcher computes them", () => {
  const few: HostSpaces[] = hostSpaces(["", "a"], ["\n1\nHome", "a\n1\nHome", "a\n2\nWork"]);
  expect(few).toEqual([
    { host: null, spaces: [{ id: "1", name: "Home" }] },
    {
      host: "a",
      spaces: [
        { id: "1", name: "Home" },
        { id: "2", name: "Work" },
      ],
    },
  ]);
  expect(switcherSections(few, "a", "zzz", []).map((x) => [x.host, x.open, x.collapsible])).toEqual(
    [
      [null, true, false],
      ["a", true, false],
    ],
  );
  const many = hostSpaces(["", "a", "b", "c", "d", "e", "f"], []);
  expect(switcherSections(many, "c", "", ["e"]).map((x) => [x.host, x.open])).toEqual([
    ["c", true],
    [null, false],
    ["a", false],
    ["b", false],
    ["d", false],
    ["e", true],
    ["f", false],
  ]);
  expect(switcherSections(many, null, "  ", []).map((x) => x.open)).toEqual([
    true,
    false,
    false,
    false,
    false,
    false,
    false,
  ]);
  // Five remotes are still few.
  expect(switcherSections(many.slice(0, 6), null, "", []).every((x) => x.open)).toBe(true);
});

test("one New space… asks where: this Mac or a host, picked in the dialog (27.8)", () => {
  const create = spyOn(transport, "createSpace").mockResolvedValue();
  show();
  // No host: no picker.
  open();
  fireEvent.click(option("New space…"));
  const dialog = () => screen.getByRole("dialog", { name: "New space" });
  expect(within(dialog()).queryByRole("listbox", { name: "Where" })).toBeNull();
  act(() => useHive.setState({ modal: null }));
  host("gpu-box");
  open();
  fireEvent.click(option("New space…"));
  const where = () => within(dialog()).getByRole("listbox", { name: "Where" });
  const mac = within(where()).getByRole("option", { name: "This Mac" });
  expect(mac.getAttribute("aria-selected")).toBe("true");
  fireEvent.change(within(dialog()).getByLabelText("Name"), { target: { value: "Train" } });
  // Picking a host shows it, the dialog and what was typed staying.
  fireEvent.click(within(where()).getByRole("option", { name: "gpu-box" }));
  expect(useHive.getState().host).toBe("gpu-box");
  expect(useHive.getState().modal).toBe("new-space");
  expect((within(dialog()).getByLabelText("Name") as HTMLInputElement).value).toBe("Train");
  fireEvent.submit(within(dialog()).getByLabelText("Name").closest("form") as HTMLFormElement);
  expect(create.mock.calls[0]?.[0]).toBe("Train");
  // Editing a space never asks where.
  act(() => useHive.setState({ modal: "edit-space" }));
  expect(screen.queryByRole("listbox", { name: "Where" })).toBeNull();
  create.mockRestore();
});
