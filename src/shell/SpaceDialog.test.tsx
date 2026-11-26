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
  gh_account: null,
  claude_account: null,
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

test("a space's Claude account is one of the accounts, kept as its folder (22.5)", () => {
  const create = spyOn(transport, "createSpace").mockResolvedValue();
  const update = spyOn(transport, "updateSpace").mockResolvedValue();
  show([home, work], "default");
  const accounts = [{ name: "Work account", config_dir: "/w" }];
  act(() => {
    const s = useHive.getState().settings;
    apply({
      type: "settings",
      settings: { ...s, claude: { ...s.claude, accounts, default_name: "Mine" } },
    });
  });
  const account = () => screen.getByRole("combobox", { name: "Claude account" });
  // New: the default account unless another is picked.
  pick("New space…");
  expect(account().textContent).toBe("Mine");
  fireEvent.mouseDown(account());
  const options = screen.getAllByRole("option").map((o) => o.textContent);
  expect(options).toEqual(["Mine", "Work account"]);
  fireEvent.click(screen.getByRole("option", { name: "Work account" }));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Other" } });
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  expect(create).toHaveBeenCalledWith("Other", { ...NO_ENV, claude_account: "/w" });
  act(() => apply({ type: "spaces", spaces: [home, work], current: "default" }));
  // A folder no account has is the default account, and saved as such.
  act(() => {
    const spaces = [home, { ...work, env: { ...work.env, claude_account: "/gone" } }];
    apply({ type: "spaces", spaces, current: "w" });
  });
  pick("Edit space…");
  expect(account().textContent).toBe("Mine");
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(update).toHaveBeenCalledWith("w", "Work", { ...work.env, claude_account: null });
  create.mockRestore();
  update.mockRestore();
});

test("the space dialog has no gh config folder field: the account brings its folder (24.13)", () => {
  const asked = spyOn(transport, "listDirs").mockResolvedValue();
  show([home], "default");
  pick("New space…");
  expect(screen.queryByLabelText("GitHub CLI config folder")).toBeNull();
  expect(screen.queryByRole("list", { name: "Folders" })).toBeNull();
  expect(asked).not.toHaveBeenCalled();
  asked.mockRestore();
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
  useHive.setState((s) => ({
    settings: {
      ...s.settings,
      notifications: { volume: 0, bounce: true, menu_bar: true, command_secs: 10 },
    },
  }));
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

/** The settings' extra gh config folders (24.13). */
const ghFolders = (config_dirs: string[]) =>
  useHive.setState((s) => ({ settings: { ...s.settings, gh: { config_dirs } } }));

test("the GitHub account is one of gh's accounts, in gh's own config or a settings' folder", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const create = spyOn(transport, "createSpace").mockResolvedValue();
  show([home], "default");
  ghFolders(["/home/user/.config/gh-ci"]);
  pick("New space…");
  expect(list.mock.calls).toEqual([[null], ["/home/user/.config/gh-ci"]]);
  // Before the answer, only gh's active account.
  expect(ghSelect().textContent).toBe("gh's active account");
  act(() => ghAccounts(null));
  expect(ghSelect().textContent).toBe("gh's active account (octo-work)");
  expect(ghOptions()).toEqual(["gh's active account (octo-work)", "octo-personal", "octo-work"]);
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "octo-personal" }));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Home" } });
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  const personal = { host: GITHUB, login: "octo-personal", config_dir: null };
  expect(create).toHaveBeenCalledWith("Home", { ...NO_ENV, gh_account: personal });

  // The other folder's accounts follow, named with their folder; picked, the account keeps it.
  act(() =>
    apply({
      type: "gh_accounts",
      gh_config_dir: "/home/user/.config/gh-ci",
      accounts: [ghLogin("octo-ci", true)],
      problem: null,
    }),
  );
  expect(ghOptions()).toEqual([
    "gh's active account (octo-work)",
    "octo-personal",
    "octo-work",
    "octo-ci (gh-ci)",
  ]);
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "octo-ci (gh-ci)" }));
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  const ci = { host: GITHUB, login: "octo-ci", config_dir: "/home/user/.config/gh-ci" };
  expect(create).toHaveBeenLastCalledWith("Home", { ...NO_ENV, gh_account: ci });
  // An active account of another folder is not gh's own active one: nothing to switch.
  expect(screen.queryByRole("button", { name: "Make active in gh…" })).toBeNull();

  // A folder's problem names it; gh's own does not.
  const problem = "No account is logged in to gh (run gh auth login)";
  act(() => ghAccounts("/home/user/.config/gh-ci", problem));
  act(() => ghAccounts(null, "gh is not installed"));
  const said = within(screen.getByRole("dialog")).getAllByRole("status");
  expect(said.map((s) => s.textContent)).toEqual([
    "gh is not installed",
    `/home/user/.config/gh-ci: ${problem}`,
  ]);
  expect(ghSelect().textContent).toBe("octo-ci (gh-ci) (not logged in)");
  // Back to gh's active account.
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "gh's active account" }));
  fireEvent.click(screen.getByRole("button", { name: "Create space" }));
  expect(create).toHaveBeenLastCalledWith("Home", NO_ENV);
  // The folders change: they are listed again.
  list.mockClear();
  act(() => ghFolders([]));
  expect(list.mock.calls).toEqual([[null]]);
  list.mockRestore();
  create.mockRestore();
});

test("an account on another host or with a bad token says so", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const gone = { host: "ghe.example", login: "gone", config_dir: null };
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
  const env = { ...NO_ENV, gh_account: { ...personal, config_dir: "/cfg" } };
  show([home, { ...work, env }], "w");
  ghFolders(["/cfg"]);
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

const WARNING = /^Commits will use git's own guess/;
const useAccount = () => screen.getByRole("button", { name: "Use the GitHub account's" });
const value = (label: string) => (screen.getByLabelText(label) as HTMLInputElement).value;

test("a GitHub account without a git identity warns, and the account's fills it", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const get = spyOn(transport, "getGhIdentity").mockResolvedValue();
  const update = spyOn(transport, "updateSpace").mockResolvedValue();
  const personal = { host: GITHUB, login: "octo-personal" };
  const workAccount = { host: GITHUB, login: "octo-work" };
  // An old answer for the same account, from before: never filled in.
  act(() =>
    apply({ type: "gh_identity", account: personal, name: "octo-personal", email: "old@x" }),
  );
  const account = { ...personal, config_dir: "/cfg" };
  show([home, { ...work, env: { ...NO_ENV, gh_account: account } }], "w");
  ghFolders(["/cfg"]);
  pick("Edit space…");
  act(() => ghAccounts("/cfg"));
  expect(screen.getByText(WARNING).textContent).toBe(
    "Commits will use git's own guess: this space has a GitHub account but no git name or email.",
  );
  expect(screen.getByText(WARNING).className).toBe("field-warning");
  expect(value("Git email")).toBe("");

  // A failure is said plainly, and it can be asked again.
  fireEvent.click(useAccount());
  expect(get).toHaveBeenLastCalledWith("/cfg", personal);
  expect((useAccount() as HTMLButtonElement).disabled).toBe(true);
  const message =
    "Could not get the identity of octo-personal on github.com: gh api user failed: HTTP 401: Bad credentials";
  act(() => apply({ type: "gh_identity_failed", account: personal, message }));
  expect(within(dialog("Edit space")).getByRole("alert").textContent).toBe(message);
  expect((useAccount() as HTMLButtonElement).disabled).toBe(false);

  // An answer for another account is not this one's.
  fireEvent.click(useAccount());
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => apply({ type: "gh_identity", account: workAccount, name: "octo-work", email: "w@x" }));
  expect(value("Git name")).toBe("");

  // A name of its own is kept; only the missing email is filled.
  fireEvent.change(screen.getByLabelText("Git name"), { target: { value: "Me" } });
  expect(screen.getByText(WARNING).textContent).toContain("but no git email.");
  const email = "1+octo-personal@users.noreply.github.com";
  act(() => apply({ type: "gh_identity", account: personal, name: "octo-personal", email }));
  expect([value("Git name"), value("Git email")]).toEqual(["Me", email]);
  expect(screen.queryByText(WARNING)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(update).toHaveBeenCalledWith("w", "Work", {
    ...NO_ENV,
    git_name: "Me",
    git_email: email,
    gh_account: account,
  });

  // Both empty: both filled.
  fireEvent.change(screen.getByLabelText("Git name"), { target: { value: "" } });
  fireEvent.change(screen.getByLabelText("Git email"), { target: { value: "" } });
  fireEvent.click(useAccount());
  act(() => apply({ type: "gh_identity", account: personal, name: "octo-personal", email }));
  expect([value("Git name"), value("Git email")]).toEqual(["octo-personal", email]);
  list.mockRestore();
  get.mockRestore();
  update.mockRestore();
});

test("no warning without an account, with a git name and email, or for another account asked", () => {
  const list = spyOn(transport, "listGhAccounts").mockResolvedValue();
  const get = spyOn(transport, "getGhIdentity").mockResolvedValue();
  show([home], "default");
  pick("Edit space…");
  act(() => ghAccounts(null));
  expect(screen.queryByText(WARNING)).toBeNull();
  // An account chosen: warned only until both are set (the email alone says the name).
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "octo-work" }));
  fireEvent.change(screen.getByLabelText("Git email"), { target: { value: "me@x" } });
  expect(screen.getByText(WARNING).textContent).toContain("but no git name.");
  // Asked for one account, then another chosen: the button is free again.
  fireEvent.click(useAccount());
  expect((useAccount() as HTMLButtonElement).disabled).toBe(true);
  fireEvent.mouseDown(ghSelect());
  fireEvent.click(screen.getByRole("option", { name: "octo-personal" }));
  expect((useAccount() as HTMLButtonElement).disabled).toBe(false);
  fireEvent.change(screen.getByLabelText("Git name"), { target: { value: "Me" } });
  expect(screen.queryByText(WARNING)).toBeNull();
  list.mockRestore();
  get.mockRestore();
});
