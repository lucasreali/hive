import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { agentPlace, notify } from "../notify";
import type { AgentState, ServiceMessage, Space } from "../protocol";
import { apply } from "../reduce";
import { goToAgent } from "../shortcuts";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const NO_ENV = {
  git_name: null,
  git_email: null,
  gh_config_dir: null,
  gh_account: null,
};
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
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
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
  expect(document.querySelector("dialog button kbd")).toBeNull();
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Job" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(update).toHaveBeenCalledWith("w", "Job", work.env);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
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
    ...agentStatus(state, null, 0, state === "working" ? null : "waiting"),
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

const GITHUB = "github.com";
const ghLogin = (login: string, active: boolean, logged_in = true, host = GITHUB) => ({
  host,
  login,
  active,
  logged_in,
});
const ghAccounts = (gh_config_dir: string | null, problem: string | null = null) =>
  apply({
    type: "gh_accounts",
    gh_config_dir,
    accounts: problem ? [] : [ghLogin("octo-personal", false), ghLogin("octo-work", true)],
    problem,
  });
const ghSelect = () => screen.getByRole("combobox", { name: "GitHub account" });
const ghOptions = () => {
  fireEvent.mouseDown(ghSelect());
  const labels = screen.getAllByRole("option").map((o) => o.textContent);
  fireEvent.keyDown(ghSelect(), { key: "Escape" });
  return labels;
};

test("the GitHub account is one of gh's accounts, listed for the dialog's gh config folder", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const create = spyOn(transport, "createSpace").mockResolvedValue();
  show([home], "default");
  pick("New space…");
  expect(list).toHaveBeenCalledWith(null);
  // Before the answer, only gh's active account.
  expect(ghSelect().textContent).toBe("gh's active account");
  act(() => ghAccounts(null));
  expect(ghSelect().textContent).toBe("gh's active account (octo-work)");
  expect(ghOptions()).toEqual(["gh's active account (octo-work)", "octo-personal", "octo-work"]);
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "octo-personal" }));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Home" } });
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  const personal = { host: GITHUB, login: "octo-personal" };
  expect(create).toHaveBeenCalledWith("Home", { ...NO_ENV, gh_account: personal });

  // Another config folder is listed once its field is left; the old answer no longer shows.
  const folder = screen.getByLabelText("GitHub CLI config folder");
  fireEvent.change(folder, { target: { value: "/cfg" } });
  expect(ghSelect().textContent).toBe("octo-personal (not logged in)");
  fireEvent.blur(folder);
  expect(list).toHaveBeenLastCalledWith("/cfg");
  act(() => ghAccounts("/cfg", "No account is logged in to gh (run gh auth login)"));
  expect(within(screen.getByRole("dialog")).getByRole("status").textContent).toBe(
    "No account is logged in to gh (run gh auth login)",
  );
  // Back to gh's active account.
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "gh's active account" }));
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  expect(create).toHaveBeenLastCalledWith("Home", { ...NO_ENV, gh_config_dir: "/cfg" });
  list.mockRestore();
  create.mockRestore();
});

test("an account on another host or with a bad token says so", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const gone = { host: "ghe.example", login: "gone" };
  show([home, { ...work, env: { ...NO_ENV, gh_account: gone } }], "w");
  pick("Edit space…");
  act(() =>
    apply({
      type: "gh_accounts",
      gh_config_dir: null,
      accounts: [ghLogin("old", false, false), ghLogin("corp", false, true, "ghe.example")],
      problem: null,
    }),
  );
  expect(ghOptions()).toEqual([
    "gh's active account",
    "old (token invalid)",
    "corp on ghe.example",
    "gone on ghe.example (not logged in)",
  ]);
  // Not listed: it cannot be made active.
  expect(screen.queryByRole("button", { name: "Make active in gh…" })).toBeNull();
  list.mockRestore();
});

test("making the space's account gh's active one asks first, over the kept dialog", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const switchGh = spyOn(transport, "switchGhAccount").mockResolvedValue();
  const personal = { host: GITHUB, login: "octo-personal" };
  const env = { ...NO_ENV, gh_config_dir: "/cfg", gh_account: personal };
  show([home, { ...work, env }], "w");
  pick("Edit space…");
  act(() => ghAccounts("/cfg"));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Job" } });
  const make = () => fireEvent.click(screen.getByRole("button", { name: "Make active in gh…" }));
  make();
  const confirm = dialog("Switch gh's active account?");
  expect(confirm.textContent).toContain(
    `octo-personal becomes gh's active account on ${GITHUB} for every shell and repository on this machine, outside Hive too.`,
  );
  // The space dialog stays open underneath, edits and all.
  expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Job");
  fireEvent.click(screen.getAllByRole("button", { name: "Cancel" })[1] as HTMLElement);
  expect(switchGh).not.toHaveBeenCalled();
  expect(useHive.getState().modal).toBe("edit-space");
  expect(screen.queryByRole("dialog", { name: "Switch gh's active account?" })).toBeNull();
  make();
  fireEvent.click(screen.getByRole("button", { name: "Switch" }));
  expect(switchGh).toHaveBeenCalledWith("/cfg", personal);
  expect(useHive.getState().modal).toBe("edit-space");
  expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Job");
  // The answer shows it active: nothing left to switch.
  act(() =>
    apply({
      type: "gh_accounts",
      gh_config_dir: "/cfg",
      accounts: [ghLogin("octo-personal", true), ghLogin("octo-work", false)],
      problem: null,
    }),
  );
  expect(screen.queryByRole("button", { name: "Make active in gh…" })).toBeNull();
  list.mockRestore();
  switchGh.mockRestore();
});
