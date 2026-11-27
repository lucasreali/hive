import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { HostLink, HostMessage } from "../protocol";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, showHost, useHive } from "../store";
import { transport } from "../transport";
import {
  HostDown,
  HostOverlay,
  HostPicker,
  HostsItem,
  HostsSettings,
  hostsSummary,
  linkText,
} from "./Hosts";
import { SettingsDialog } from "./SettingsDialog";

// 26.18: a remote host's link in the interface: its chip, the sidebar's note and the terminals'
// overlay while it is down, and Settings → Hosts.

const link = (host: string, state: HostLink) =>
  act(() => apply({ type: "host_link", host, ...state } as HostMessage));

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

/** A link as the store keeps it. */
const at = (link: HostLink) => ({ ...link, since: 0 });
const connected = at({ state: "connected" });
const reconnecting = at({ state: "reconnecting", reason: "gone", retry_secs: 5 });
const refused = at({ state: "disconnected", reason: "denied", kind: "auth" });
const mismatched = at({ state: "disconnected", reason: "version", kind: "version" });
const installing = at({ state: "installing", cpu: "x86_64" });

test("the status bar's summary for each mix of states (27.8)", () => {
  const text = (links: Record<string, ReturnType<typeof at>>, local = "connected" as const) =>
    hostsSummary(local, links);
  expect(text({ a: connected })).toEqual({ text: "1 host", status: "connected", warning: false });
  expect(text({ a: reconnecting }).text).toBe("1 host down");
  expect(text({ a: at({ state: "connecting" }) }).text).toBe("1 host connecting");
  expect(text({ a: connected, b: connected, c: connected }).text).toBe("3 hosts");
  expect(text({ a: connected, b: refused, c: connected })).toEqual({
    text: "1 of 3 hosts down",
    status: "disconnected",
    warning: true,
  });
  expect(text({ a: mismatched, b: reconnecting, c: connected }).text).toBe("2 of 3 hosts down");
  expect(text({ a: installing, b: connected, c: connected })).toEqual({
    text: "1 of 3 hosts connecting",
    status: "connecting",
    warning: false,
  });
  // A host down says more than one coming up.
  expect(text({ a: installing, b: refused }).text).toBe("1 of 2 hosts down");
  // This Mac's own service first.
  expect(hostsSummary("connecting", { a: refused })).toEqual({
    text: "This Mac connecting",
    status: "connecting",
    warning: false,
  });
  expect(hostsSummary("disconnected", { a: connected })).toEqual({
    text: "This Mac disconnected",
    status: "disconnected",
    warning: true,
  });
  expect(hostsSummary("version_mismatch", {}).text).toBe("This Mac version mismatch");
});

test("the host item opens every host's link, problems first, with their actions (27.8)", () => {
  const reconnect = spyOn(transport, "reconnectHost").mockResolvedValue();
  const restart = spyOn(transport, "restartRemote").mockResolvedValue();
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  render(<HostsItem />);
  link("gpu-box", { state: "connected" });
  link("lab", { state: "installing", cpu: "aarch64" });
  const item = screen.getByRole("button", { name: "1 of 2 hosts connecting" });
  fireEvent.click(item);
  expect(screen.getByRole("dialog", { name: "Hosts" }).textContent).toContain(
    "labinstalling Hive (aarch64)",
  );
  fireEvent.click(item);
  link("lab", { state: "reconnecting", reason: "Connection closed", retry_secs: 5 });
  expect(item.textContent).toBe("1 of 2 hosts down");
  expect(item.dataset.warning).toBe("true");
  fireEvent.click(item);
  expect(item.getAttribute("aria-expanded")).toBe("true");
  const popover = screen.getByRole("dialog", { name: "Hosts" });
  const rows = () => [...popover.querySelectorAll("li")].map((r) => r.textContent);
  expect(rows()[0]).toMatch(/^labreconnecting since \d+sReconnect$/);
  expect(rows().slice(1)).toEqual(["This Macconnectedshown", "gpu-boxconnected"]);
  // The focus goes in.
  expect(document.activeElement?.textContent).toBe("Reconnect");
  // ↑/↓ go through its buttons (WebKit's Tab skips them), stopping at either end.
  const step = (key: string) => fireEvent.keyDown(document.activeElement as Element, { key });
  step("ArrowUp");
  expect(document.activeElement?.textContent).toBe("Reconnect");
  step("ArrowDown");
  expect(document.activeElement?.textContent).toBe("Hosts settings…");
  step("ArrowDown");
  expect(document.activeElement?.textContent).toBe("Hosts settings…");
  step("ArrowUp");
  expect(document.activeElement?.textContent).toBe("Reconnect");
  fireEvent.click(screen.getByRole("button", { name: "Reconnect lab" }));
  expect(reconnect).toHaveBeenCalledWith("lab");
  // Another version (Q8): restart it, ending its terminals.
  const version = { version: "0.28.0", app_version: "0.29.1", terminals: 2 };
  link("gpu-box", { state: "disconnected", reason: "version", kind: "version", ...version });
  act(() => showHost("gpu-box"));
  expect(rows()).toContain(
    "gpu-boxdisconnected: version 0.28.0, this app 0.29.1: restart the remote service to update itshownRestart the remote service",
  );
  const button = screen.getByRole("button", { name: "Restart the remote service on gpu-box" });
  expect(button.title).toBe("Ends its 2 terminals");
  fireEvent.click(button);
  expect(restart).toHaveBeenCalledWith("gpu-box");
  link("gpu-box", { state: "disconnected", reason: "version", kind: "version", terminals: 1 });
  expect(screen.getByRole("button", { name: "Restart the remote service on gpu-box" }).title).toBe(
    "Ends its terminal",
  );
  // Esc closes it, back to the item; so do a second click and a click outside.
  fireEvent.keyDown(popover, { key: "x" });
  expect(screen.getByRole("dialog", { name: "Hosts" })).toBeDefined();
  fireEvent.keyDown(popover, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(item);
  fireEvent.click(item);
  fireEvent.mouseDown(screen.getByRole("dialog", { name: "Hosts" }));
  fireEvent.mouseDown(item);
  expect(screen.getByRole("dialog", { name: "Hosts" })).toBeDefined();
  fireEvent.click(item);
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(item);
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("dialog")).toBeNull();
  // Esc with the focus nowhere closes it too.
  fireEvent.click(item);
  fireEvent.keyDown(document.body, { key: "a" });
  expect(screen.getByRole("dialog", { name: "Hosts" })).toBeDefined();
  fireEvent.keyDown(document.body, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  // Settings → Hosts from it.
  fireEvent.click(item);
  fireEvent.click(screen.getByRole("button", { name: "Hosts settings…" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useHive.getState().modal).toBe("settings");
  render(<SettingsDialog />);
  expect(screen.getByRole("button", { name: "Hosts" }).getAttribute("aria-current")).toBe("page");
  reconnect.mockRestore();
  restart.mockRestore();
});

test("with many hosts the popover filters them; this Mac shows its own trouble (27.8)", () => {
  render(<HostsItem />);
  for (const host of ["a1", "a2", "a3", "b1", "b2", "b3"]) link(host, { state: "connected" });
  fireEvent.click(screen.getByRole("button", { name: "This Mac connecting" }));
  const popover = screen.getByRole("dialog", { name: "Hosts" });
  const rows = () => [...popover.querySelectorAll("li")].map((r) => r.textContent);
  expect(rows()).toHaveLength(7);
  const filter = screen.getByRole("searchbox", { name: "Filter hosts" });
  expect(document.activeElement).toBe(filter);
  fireEvent.change(filter, { target: { value: " B " } });
  expect(rows()).toEqual(["b1connected", "b2connected", "b3connected"]);
  fireEvent.change(filter, { target: { value: "zzz" } });
  expect(rows()).toEqual(["No host matches"]);
  fireEvent.change(filter, { target: { value: "mac" } });
  act(() => apply({ type: "disconnected", reason: "gone", bundled: false }));
  link("b2", { state: "disconnected", reason: "denied", kind: "auth" });
  fireEvent.change(filter, { target: { value: "" } });
  // This Mac's trouble and b2's both first; b2 refused has Reconnect.
  expect(rows().slice(0, 2)).toEqual([
    "This Macdisconnectedshown",
    "b2disconnected: deniedReconnect",
  ]);
  act(() =>
    apply({
      type: "version_mismatch",
      protocol: 1,
      version: "0.1.0",
      app_protocol: 2,
      app_version: "0.2.0",
    } as HostMessage),
  );
  expect(rows()[0]).toBe("This Macversion mismatchshown");
});

test("the new space's host picker: this Mac or a host, by click, keys or filter (27.8)", () => {
  const pick = (value: string | null) => {
    const onChange = spyOn({ f: (_: string | null) => {} }, "f");
    cleanup();
    render(<HostPicker value={value} onChange={onChange} />);
    return onChange;
  };
  link("gpu-box", { state: "connected" });
  link("lab", { state: "reconnecting", reason: "gone", retry_secs: 5 });
  link("box", { state: "connected" });
  let onChange = pick(null);
  const options = () =>
    screen.getAllByRole("option").map((o) => `${o.getAttribute("aria-selected")} ${o.textContent}`);
  expect(options()).toEqual(["true This Mac", "false gpu-box", "false lab", "false box"]);
  expect(screen.getByRole("option", { name: "lab" }).getAttribute("aria-disabled")).toBe("true");
  expect(screen.getByRole("option", { name: "lab" }).title).toBe("lab: reconnecting");
  expect(screen.queryByRole("searchbox")).toBeNull();
  // A host down takes none; the one picked already changes nothing.
  fireEvent.click(screen.getByRole("option", { name: "lab" }));
  fireEvent.click(screen.getByRole("option", { name: "This Mac" }));
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("option", { name: "box" }));
  expect(onChange).toHaveBeenCalledWith("box");
  const list = () => screen.getByRole("listbox", { name: "Where" });
  // ↑ at the top stays; ↓ goes to the next one that can take a space.
  fireEvent.keyDown(list(), { key: "ArrowUp" });
  fireEvent.keyDown(list(), { key: "a" });
  expect(onChange).toHaveBeenCalledTimes(1);
  onChange = pick("gpu-box");
  expect(list().getAttribute("aria-activedescendant")).toBe("space-host-gpu-box");
  fireEvent.keyDown(list(), { key: "ArrowDown" });
  expect(onChange).toHaveBeenCalledWith("box");
  onChange = pick("box");
  fireEvent.keyDown(list(), { key: "ArrowDown" });
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.keyDown(list(), { key: "ArrowUp" });
  expect(onChange).toHaveBeenCalledWith("gpu-box");
  // With many hosts, a filter, whose ↓ moves too.
  for (const host of ["c1", "c2", "c3"]) link(host, { state: "connected" });
  onChange = pick(null);
  const filter = screen.getByRole("searchbox", { name: "Filter hosts" });
  fireEvent.change(filter, { target: { value: "c" } });
  expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
    "This Mac",
    "c1",
    "c2",
    "c3",
  ]);
  fireEvent.keyDown(filter, { key: "ArrowDown" });
  expect(onChange).toHaveBeenCalledWith("c1");
  fireEvent.change(filter, { target: { value: "zzz" } });
  expect(screen.getByText("No host matches")).toBeDefined();
});

test("Settings → Hosts filters a long list (27.8)", () => {
  const hosts = ["a1", "a2", "a3", "b1", "b2", "b3"];
  const settings = { ...DEFAULT_SETTINGS, remote: { hosts, keep_minutes: 60 } };
  act(() => apply({ type: "settings", settings }));
  render(<HostsSettings />);
  const names = () => screen.queryAllByRole("button", { name: /^Remove / }).map((b) => b.title);
  expect(names()).toHaveLength(6);
  expect(document.querySelector(".hosts")?.hasAttribute("data-many")).toBe(true);
  fireEvent.change(screen.getByRole("searchbox", { name: "Filter hosts" }), {
    target: { value: "a" },
  });
  expect(names()).toEqual(["Remove a1", "Remove a2", "Remove a3"]);
  fireEvent.change(screen.getByRole("searchbox", { name: "Filter hosts" }), {
    target: { value: "zzz" },
  });
  expect(screen.getByText("No host matches.")).toBeDefined();
});

test("Settings → Hosts offers the popover's action on a host with a problem (27.10)", () => {
  const reconnect = spyOn(transport, "reconnectHost").mockResolvedValue();
  const restart = spyOn(transport, "restartRemote").mockResolvedValue();
  const hosts = ["gpu-box", "lab", "vps", "box"];
  const settings = { ...DEFAULT_SETTINGS, remote: { hosts, keep_minutes: 60 } };
  act(() => apply({ type: "settings", settings }));
  link("gpu-box", { state: "connected" });
  link("lab", { state: "reconnecting", reason: "gone", retry_secs: 5 });
  const version = { version: "0.30.0", app_version: "0.31.0", terminals: 3 };
  link("vps", { state: "disconnected", reason: "version", kind: "version", ...version });
  link("box", { state: "disconnected", reason: "Permission denied.", kind: "auth" });
  render(<HostsSettings />);
  const actions = () =>
    [...document.querySelectorAll(".host-row-actions")].map((a) => a.textContent);
  // Nothing on a connected host; Reconnect while down or refused; a restart on a mismatch.
  expect(actions()).toEqual(["", "Reconnect", "Restart the remote service", "Reconnect"]);
  expect(document.querySelector(".hosts")?.textContent).toContain(
    "vpsdisconnected: version 0.30.0, this app 0.31.0: restart the remote service to update it",
  );
  const button = screen.getByRole("button", { name: "Restart the remote service on vps" });
  expect(button.title).toBe("Ends its 3 terminals");
  fireEvent.click(button);
  expect(restart).toHaveBeenCalledWith("vps");
  fireEvent.click(screen.getByRole("button", { name: "Reconnect lab" }));
  fireEvent.click(screen.getByRole("button", { name: "Reconnect box" }));
  expect(reconnect.mock.calls).toEqual([["lab"], ["box"]]);
  // Once linked again, the action goes.
  link("vps", { state: "connected" });
  expect(screen.queryByRole("button", { name: "Restart the remote service on vps" })).toBeNull();
  reconnect.mockRestore();
  restart.mockRestore();
});

test("a link's text for each state", () => {
  const since = 0;
  expect(linkText({ state: "removed", since })).toBe("removed");
  expect(linkText({ state: "reconnecting", reason: "x", retry_secs: 1, since })).toBe(
    "reconnecting",
  );
  expect(linkText({ state: "disconnected", reason: "x", kind: "version", since })).toBe(
    "disconnected: version ?, this app ?: restart the remote service to update it",
  );
});

test("while the shown host is down, the sidebar and its terminals say so", () => {
  const reconnect = spyOn(transport, "reconnectHost").mockResolvedValue();
  render(
    <>
      <HostDown />
      <HostOverlay />
    </>,
  );
  link("gpu-box", { state: "connected" });
  act(() => showHost("gpu-box"));
  expect(screen.queryByRole("status")).toBeNull();
  link("gpu-box", { state: "reconnecting", reason: "gone", retry_secs: 1 });
  const notes = screen.getAllByRole("status").map((n) => n.textContent);
  expect(notes).toEqual(["Reconnecting to gpu-box…Reconnect", "Reconnecting to gpu-box…"]);
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(reconnect).toHaveBeenCalledWith("gpu-box");
  // This Mac's spaces are never down.
  act(() => showHost(null));
  expect(screen.queryByRole("status")).toBeNull();
  reconnect.mockRestore();
});

test("Settings → Hosts adds, tests and removes a host by its alias", () => {
  const saved = spyOn(transport, "setSettings").mockResolvedValue();
  const reconnect = spyOn(transport, "reconnectHost").mockResolvedValue();
  const settings = { ...DEFAULT_SETTINGS, remote: { hosts: ["gpu-box", "lab"], keep_minutes: 60 } };
  act(() => apply({ type: "settings", settings }));
  link("gpu-box", { state: "connected" });
  render(<HostsSettings />);
  expect(document.querySelector(".hosts")?.textContent).toBe(
    "gpu-boxconnectedTestlabnot linked yetTest",
  );
  fireEvent.click(screen.getAllByRole("button", { name: "Test" })[0] as HTMLElement);
  expect(reconnect).toHaveBeenCalledWith("gpu-box");
  // Nothing to add until an alias is typed.
  const add = screen.getByRole("button", { name: "Add host" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  fireEvent.submit(add.closest("form") as HTMLFormElement);
  expect(saved).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText("Host alias"), { target: { value: " box " } });
  fireEvent.submit(add.closest("form") as HTMLFormElement);
  expect(saved.mock.calls[0]?.[0].remote.hosts).toEqual(["gpu-box", "lab", "box"]);
  expect((screen.getByLabelText("Host alias") as HTMLInputElement).value).toBe("");
  act(() => apply({ type: "settings", settings }));
  fireEvent.click(screen.getByRole("button", { name: "Remove lab" }));
  expect(saved.mock.calls[1]?.[0].remote.hosts).toEqual(["gpu-box"]);
  saved.mockRestore();
  reconnect.mockRestore();
});
