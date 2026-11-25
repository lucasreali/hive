import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { HostLink, HostMessage } from "../protocol";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, showHost, useHive } from "../store";
import { transport } from "../transport";
import { HostChips, HostDown, HostOverlay, HostsSettings, linkText } from "./Hosts";

// 26.18: a remote host's link in the interface: its chip, the sidebar's note and the terminals'
// overlay while it is down, and Settings → Hosts.

const link = (host: string, state: HostLink) =>
  act(() => apply({ type: "host_link", host, ...state } as HostMessage));

beforeEach(() => useHive.setState(initialState, true));
afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

test("each link's chip says its state, with Reconnect while it is down", () => {
  const reconnect = spyOn(transport, "reconnectHost").mockResolvedValue();
  const restart = spyOn(transport, "restartRemote").mockResolvedValue();
  render(<HostChips />);
  link("gpu-box", { state: "connecting" });
  link("lab", { state: "installing", cpu: "aarch64" });
  const chips = () => [...document.querySelectorAll(".host-chip")].map((c) => c.textContent);
  expect(chips()).toEqual(["gpu-boxconnecting", "labinstalling Hive (aarch64)"]);
  link("gpu-box", { state: "reconnecting", reason: "Connection closed", retry_secs: 5 });
  expect(chips()[0]).toMatch(/^gpu-boxreconnecting since \d+sReconnect$/);
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(reconnect).toHaveBeenCalledWith("gpu-box");
  link("gpu-box", { state: "disconnected", reason: "add the key to your ssh agent", kind: "auth" });
  expect(chips()[0]).toBe("gpu-boxdisconnected: add the key to your ssh agentReconnect");
  // Another version (Q8): restart it, ending its terminals, or stay disconnected.
  const version = { version: "0.28.0", app_version: "0.29.1", terminals: 2 };
  link("lab", { state: "disconnected", reason: "version", kind: "version", ...version });
  expect(chips()[1]).toBe(
    "labdisconnected: version 0.28.0, this app 0.29.1Restart the remote service",
  );
  const button = screen.getByRole("button", { name: "Restart the remote service" });
  expect(button.title).toBe("Ends its 2 terminals");
  fireEvent.click(button);
  expect(restart).toHaveBeenCalledWith("lab");
  link("lab", { state: "disconnected", reason: "version", kind: "version", terminals: 1 });
  expect(screen.getByRole("button", { name: "Restart the remote service" }).title).toBe(
    "Ends its terminal",
  );
  link("lab", { state: "connected" });
  expect(chips()[1]).toBe("labconnected");
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
    "disconnected: version ?, this app ?",
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
