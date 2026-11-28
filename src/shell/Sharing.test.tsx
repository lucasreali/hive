import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { HostMessage, Sharing, SharingState } from "../protocol";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { transport } from "../transport";
import { SharingSettings } from "./Sharing";

// 29.12: Settings → Sharing, a remote host's own Hive shared with teammates. The host's service
// checks and saves; the section edits a draft and sends it whole.

const OFF: Sharing = { on: false, group: "", spaces: [], people: [] };

const state = (over: Partial<SharingState> = {}): SharingState => ({
  sharing: OFF,
  owner: "alice",
  groups: ["alice", "team"],
  spaces: [
    { id: "default", name: "Default" },
    { id: "work", name: "Work" },
  ],
  unavailable: null,
  error: null,
  ...over,
});

/** `host`'s service answers. */
const answer = (host: string, over: Partial<SharingState> = {}) =>
  act(() => apply({ type: "sharing", host, ...state(over) } as HostMessage));

const hosts = (names: string[]) =>
  act(() =>
    apply({
      type: "settings",
      settings: { ...DEFAULT_SETTINGS, remote: { hosts: names, keep_minutes: 60, joined: [] } },
    }),
  );

const linked = (host: string) =>
  act(() => apply({ type: "host_link", host, state: "connected" } as HostMessage));

const pick = (name: string, option: string) => {
  fireEvent.mouseDown(screen.getByRole("combobox", { name }));
  fireEvent.click(screen.getByRole("option", { name: option }));
};

let ask: ReturnType<typeof spyOn>;
let save: ReturnType<typeof spyOn>;
beforeEach(() => {
  useHive.setState(initialState, true);
  ask = spyOn(transport, "getSharing").mockResolvedValue();
  save = spyOn(transport, "setSharing").mockResolvedValue();
});
afterEach(() => {
  cleanup();
  ask.mockRestore();
  save.mockRestore();
  useHive.setState(initialState, true);
});

test("without a remote host, or with it down, there is nothing to share yet", () => {
  render(<SharingSettings />);
  expect(screen.getByText(/Add a remote host in Hosts first/)).toBeDefined();
  hosts(["gpu-box"]);
  expect(screen.getByText("gpu-box is not connected.")).toBeDefined();
  expect(ask).not.toHaveBeenCalled();
  // Linked: its service is asked, and the section waits for it.
  linked("gpu-box");
  expect(ask).toHaveBeenCalledWith("gpu-box");
  expect(screen.getByText("Asking gpu-box…")).toBeDefined();
  // A service that cannot be shared says why.
  answer("gpu-box", { unavailable: "Only the Hive of a remote Linux host can be shared" });
  expect(screen.getByText("Only the Hive of a remote Linux host can be shared.")).toBeDefined();
  expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
});

test("a draft is edited and saved whole; a refusal keeps it and says why", () => {
  hosts(["gpu-box"]);
  linked("gpu-box");
  answer("gpu-box", { error: "refusing /tmp/hive-shared-1000: someone else made it first" });
  render(<SharingSettings />);
  expect(screen.getByText(/your login name/).textContent).toContain("alice");
  expect(screen.getByRole("alert").textContent).toBe(
    "Not shared: refusing /tmp/hive-shared-1000: someone else made it first",
  );
  const saveButton = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
  expect(saveButton.disabled).toBe(true);
  expect(screen.getByText("Nobody can join yet.")).toBeDefined();
  fireEvent.click(screen.getByRole("checkbox", { name: "Share this Hive" }));
  pick("Unix group", "team");
  fireEvent.click(screen.getByRole("checkbox", { name: "Work" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Default" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Default" }));
  fireEvent.click(screen.getByRole("button", { name: "Add person" }));
  fireEvent.click(screen.getByRole("button", { name: "Add person" }));
  const type = (label: string, value: string) =>
    fireEvent.change(screen.getByLabelText(label), { target: { value } });
  type("Login name 1", "bob");
  type("Display name 1", "Bob");
  type("Git name 1", "Bob B");
  type("Git e-mail 1", "bob@example.com");
  pick("Role 1", "Collaborator");
  type("Login name 2", "carol");
  // An empty row is named by its place, a named one by its login.
  expect(screen.getByRole("button", { name: "Remove bob" })).toBeDefined();
  fireEvent.click(screen.getByRole("button", { name: "Remove carol" }));
  expect(saveButton.disabled).toBe(false);
  fireEvent.click(saveButton);
  const sent: Sharing = {
    on: true,
    group: "team",
    spaces: ["work"],
    people: [
      {
        login: "bob",
        display: "Bob",
        git_name: "Bob B",
        git_email: "bob@example.com",
        role: "collaborator",
      },
    ],
  };
  expect(save).toHaveBeenCalledWith("gpu-box", sent);
  // Refused: the draft stays, with why.
  act(() => apply({ type: "sharing_failed", host: "gpu-box", message: "There is no user bob" }));
  expect(screen.getByText("There is no user bob")).toBeDefined();
  expect((screen.getByLabelText("Login name 1") as HTMLInputElement).value).toBe("bob");
  // Saved: the form shows what the service saved, nothing left to save.
  answer("gpu-box", { sharing: sent });
  expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.queryByText("There is no user bob")).toBeNull();
  // A person with no login yet is named by their row.
  fireEvent.click(screen.getByRole("button", { name: "Add person" }));
  expect(screen.getByRole("button", { name: "Remove person 2" })).toBeDefined();
});

test("with several hosts one is picked; each keeps its own answer", () => {
  hosts(["gpu-box", "lab"]);
  linked("gpu-box");
  linked("lab");
  answer("lab", { owner: "lab-owner" });
  render(<SharingSettings />);
  expect(ask).toHaveBeenCalledWith("gpu-box");
  expect(screen.getByText("Asking gpu-box…")).toBeDefined();
  pick("Host", "lab");
  expect(ask).toHaveBeenCalledWith("lab");
  expect(screen.getByText(/your login name/).textContent).toContain("lab-owner");
  // A refusal for a host never asked changes nothing.
  act(() => apply({ type: "sharing_failed", host: "other", message: "no" }));
  expect(useHive.getState().sharing.other).toBeUndefined();
  // The host picked is removed: the first one shows again.
  hosts(["gpu-box"]);
  expect(screen.getByText("Asking gpu-box…")).toBeDefined();
});
