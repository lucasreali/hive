import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { agentPlace, notify } from "../notify";
import { goToAgent } from "../shortcuts";
import {
  type AgentState,
  apply,
  initialState,
  type ServiceMessage,
  type Space,
  useHive,
} from "../store";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const NO_ENV = { claude_config_dir: null, git_name: null, git_email: null, gh_config_dir: null };
const home: Space = { id: "default", name: "Home", projects: [shop.id], env: NO_ENV };
const work: Space = {
  id: "w",
  name: "Work",
  projects: [api.id],
  env: { ...NO_ENV, git_email: "me@work" },
};
const empty: Space = { id: "e", name: "Empty", projects: [], env: NO_ENV };

/** The app with both projects, in `spaces` with `current` shown. */
function show(spaces: Space[], current: string) {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  act(() => apply({ type: "spaces", spaces, current }));
}

const projectRows = () =>
  [...document.querySelectorAll(".tree-row.project .label")].map((r) => r.textContent);
const picker = () => screen.getByRole("combobox", { name: "Space" });
const pick = (label: string) => {
  fireEvent.mouseDown(picker());
  fireEvent.click(screen.getByRole("option", { name: label }));
};
const dialog = (name: string) => screen.getByRole("dialog", { name }) as HTMLDialogElement;

test("the sidebar shows the current space's projects; the select picks another", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop, api] }));
  // Every project until the service sent the spaces, and no select.
  expect(projectRows()).toEqual(["shop", "api"]);
  expect(screen.queryByRole("combobox", { name: "Space" })).toBeNull();
  act(() => apply({ type: "spaces", spaces: [home, work], current: "w" }));
  expect(projectRows()).toEqual(["api"]);
  expect(picker().textContent).toBe("Work");
  pick("Home");
  expect(select).toHaveBeenCalledWith("default");
  select.mockRestore();
});

test("a new space is created with its environment, and a refusal is shown", () => {
  const create = spyOn(transport, "createSpace").mockResolvedValue();
  show([home], "default");
  pick("New space…");
  const box = dialog("New space");
  expect(box.open).toBe(true);
  expect(screen.queryByRole("button", { name: "Delete space" })).toBeNull();
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Work" } });
  const email = screen.getByLabelText("Git email");
  fireEvent.change(email, { target: { value: "me@work" } });
  fireEvent.change(email, { target: { value: "me@home" } });
  fireEvent.change(screen.getByLabelText("Git name"), { target: { value: "x" } });
  fireEvent.change(screen.getByLabelText("Git name"), { target: { value: "" } });
  fireEvent.click(screen.getByRole("button", { name: "Create space Enter" }));
  expect(create).toHaveBeenCalledWith("Work", { ...NO_ENV, git_email: "me@home" });
  act(() => apply({ type: "space_failed", message: "The name is too long" }));
  expect(screen.getByRole("alert").textContent).toBe("The name is too long");
  // The answer closes it.
  act(() => apply({ type: "spaces", spaces: [home, work], current: "w" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  create.mockRestore();
});

test("the current space is edited; only an empty one can be deleted", () => {
  const update = spyOn(transport, "updateSpace").mockResolvedValue();
  const remove = spyOn(transport, "deleteSpace").mockResolvedValue();
  show([home, work], "w");
  pick("Edit space…");
  dialog("Edit space");
  expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Work");
  expect((screen.getByLabelText("Git email") as HTMLInputElement).value).toBe("me@work");
  const del = screen.getByRole("button", { name: "Delete space" }) as HTMLButtonElement;
  expect(del.disabled).toBe(true);
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Job" } });
  fireEvent.click(screen.getByRole("button", { name: "Save Enter" }));
  expect(update).toHaveBeenCalledWith("w", "Job", work.env);
  fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" }));
  expect(screen.queryByRole("dialog")).toBeNull();

  act(() => apply({ type: "spaces", spaces: [home, work, empty], current: "e" }));
  pick("Edit space…");
  fireEvent.click(screen.getByRole("button", { name: "Delete space" }));
  expect(remove).toHaveBeenCalledWith("e");
  update.mockRestore();
  remove.mockRestore();
});

test("going to an agent of another space makes its space current", () => {
  const select = spyOn(transport, "selectSpace").mockResolvedValue();
  show([home, work], "default");
  const agent = { id: "a", terminal: 1, project: api.id, worktree: api.id, cwd: api.path };
  act(() => apply({ type: "agent_detected", channel: 1, ...agent }));
  goToAgent(useHive.getState().agents.a);
  expect(select).toHaveBeenCalledWith("w");
  expect(useHive.getState().selection).toBe("a");
  // One in the current space changes no space.
  select.mockClear();
  act(() => apply({ type: "spaces", spaces: [home, work], current: "w" }));
  goToAgent(useHive.getState().agents.a);
  expect(select).not.toHaveBeenCalled();
  select.mockRestore();
});

test("an alert names the agent's space when there are several", () => {
  show([home], "default");
  const agent = { id: "a", terminal: 1, project: api.id, worktree: api.id, cwd: api.path };
  act(() => apply({ type: "agent_detected", channel: 1, ...agent }));
  expect(agentPlace(useHive.getState(), "a")).toBe("api · main");
  act(() => apply({ type: "spaces", spaces: [home, work], current: "default" }));
  expect(agentPlace(useHive.getState(), "a")).toBe("Work · api · main");
});

test("inbox items name their agent's space when there are several", () => {
  show([home, work], "default");
  const agent = { id: "a", terminal: 1, project: api.id, worktree: api.id, cwd: api.path };
  act(() => apply({ type: "agent_detected", channel: 1, ...agent }));
  const status = (state: AgentState): ServiceMessage => ({
    type: "agent_state",
    id: "a",
    ...agentStatus(state),
    subagents: [],
  });
  act(() => apply(status("working")));
  // Muted: no tone in the test DOM.
  useHive.setState((s) => ({ settings: { ...s.settings, notifications: { volume: 0 } } }));
  act(() => {
    notify(status("waiting_permission"));
    apply(status("waiting_permission"));
  });
  expect(useHive.getState().inbox[0]?.space).toBe("Work");
  fireEvent.click(screen.getByRole("button", { name: /pending/ }));
  const texts = screen.getAllByRole("menuitem").map((i) => i.textContent);
  expect(texts[0]).toContain("Work · waiting for permission");
  expect(texts[1]).toMatch(/Work · \d+s ago$/);
});
