import { afterAll, afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { onMessage } from "./connect";
import { agentKey } from "./hosts";
import { goToClicked, notify, SUMMARY_MS, spaceName, summarize, terminalName } from "./notify";
import type { HostLink, HostMessage, Project } from "./protocol";
import { apply } from "./reduce";
import { goToAgent } from "./shortcuts";
import {
  activateTab,
  addTab,
  DEFAULT_SETTINGS,
  hostView,
  initialState,
  linkDown,
  onRemote,
  showHost,
  useHive,
} from "./store";
import { transport } from "./transport";

// 26.18: a remote host's link in the store (`host_link`), the host named in alerts, and one
// summary after a reattach.

const HOST = "gpu-box";
const link = (state: HostLink) => onMessage({ type: "host_link", host: HOST, ...state });
const remote = (m: object) => onMessage({ ...m, host: HOST } as HostMessage);

const models: Project = {
  id: "/m",
  name: "models",
  path: "/m",
  worktrees: [
    {
      id: "/m",
      name: "main",
      path: "/m",
      branch: "main",
      main: true,
      claude: false,
      status: null,
    },
  ],
  error: null,
};
const space = (id: string, name: string, projects: string[]) => ({
  id,
  name,
  projects,
  env: { git_name: null, git_email: null, gh_account: null, claude_account: null },
});
const state = (id: string, s: string, pending: boolean) => ({
  type: "agent_state",
  id,
  state: s,
  urgency: 3,
  pending,
  interrupted: false,
  alert: pending ? "attention" : null,
  notify: pending,
  subagents: [],
  activity: null,
  since_ms: 1,
});

let shown: { title: string; body: string; agent: string }[] = [];
const show = spyOn(transport, "showNotification").mockImplementation(async (title, body, agent) => {
  shown.push({ title, body, agent });
});
afterAll(() => show.mockRestore());

const reset = () => {
  useHive.setState(initialState, true);
  useHive.setState({
    settings: {
      ...DEFAULT_SETTINGS,
      notifications: { ...DEFAULT_SETTINGS.notifications, volume: 0 },
    },
  });
  shown = [];
};
beforeEach(reset);
afterEach(() => {
  link({ state: "removed" });
  useHive.setState(initialState, true);
});

/** The host linked, with its space "Train" holding `models` and agent `a` there. */
function linked() {
  link({ state: "connected" });
  remote({ type: "spaces", spaces: [space("t", "Train", ["/m"])], current: "t" });
  remote({ type: "projects", projects: [models] });
  remote({ type: "agent_detected", channel: 2, id: "a", project: "/m", worktree: "/m", cwd: "/m" });
}

test("a link's state is kept with since when; a new state starts it again", () => {
  const now = spyOn(Date, "now").mockReturnValue(1000);
  link({ state: "connecting" });
  expect(useHive.getState().links[HOST]).toEqual({ state: "connecting", since: 1000 });
  now.mockReturnValue(2000);
  const reason = "Connection closed";
  link({ state: "reconnecting", reason, retry_secs: 1 });
  now.mockReturnValue(3000);
  link({ state: "reconnecting", reason, retry_secs: 2 });
  expect(useHive.getState().links[HOST]).toMatchObject({ retry_secs: 2, since: 2000 });
  expect(linkDown(useHive.getState(), HOST)).toBe(true);
  expect(linkDown(useHive.getState(), undefined)).toBe(false);
  link({ state: "connected" });
  expect(linkDown(useHive.getState(), HOST)).toBe(false);
  now.mockRestore();
});

test("a host removed from the settings goes with its data; while shown, this Mac's shows", () => {
  linked();
  showHost(HOST);
  expect(onRemote(useHive.getState())).toBe(true);
  expect(useHive.getState().currentSpace).toBe("t");
  link({ state: "removed" });
  const s = useHive.getState();
  expect(s.host).toBeNull();
  expect(s.links).toEqual({});
  expect(s.stash).toEqual({});
  // Not shown: it goes too, and the shown host stays.
  linked();
  link({ state: "removed" });
  expect(useHive.getState().stash[HOST]).toBeUndefined();
});

test("switching hosts keeps each one's place in its space", () => {
  linked();
  apply({ type: "spaces", spaces: [space("default", "Default", [])], current: "default" });
  showHost(HOST);
  // Its first project shows; the place picked there comes back after a visit to this Mac.
  expect(useHive.getState().selection).toBe("/m");
  useHive.setState({ selection: "/m" });
  showHost(null);
  expect(useHive.getState().selection).toBeNull();
  showHost(HOST);
  expect(useHive.getState().selection).toBe("/m");
  showHost(HOST);
  expect(useHive.getState().host).toBe(HOST);
});

test("going to a remote agent or terminal shows its host first", () => {
  linked();
  addTab(2, "/m", HOST);
  expect(useHive.getState().activeTab).toBeNull();
  const tab = useHive.getState().tabs[0];
  activateTab(tab as never);
  expect(useHive.getState()).toMatchObject({ host: HOST, activeTab: 2, selection: "/m" });
  showHost(null);
  const agent = hostView(useHive.getState(), HOST).agents[agentKey(HOST, "a")];
  goToAgent(agent as never);
  expect(useHive.getState()).toMatchObject({ host: HOST, selection: agentKey(HOST, "a") });
  showHost(null);
  goToClicked(agentKey(HOST, "a"));
  expect(useHive.getState().host).toBe(HOST);
});

test("alerts name the host: the space and host, the terminal on its host", () => {
  linked();
  const key = agentKey(HOST, "a");
  expect(spaceName(useHive.getState(), key)).toBe("Train · gpu-box");
  remote(state("a", "waiting_permission", true));
  expect(shown).toEqual([
    {
      title: "Claude is waiting for permission",
      body: "Train · gpu-box · models · main",
      agent: key,
    },
  ]);
  const item = useHive.getState().inbox[0];
  expect(item).toMatchObject({ agent: key, host: HOST, space: "Train · gpu-box" });
  // A terminal's: its worktree's name on its host.
  addTab(2, "/m", HOST);
  remote({ type: "terminal_opened", channel: 2, worktree: "/m", account: "Default" });
  expect(terminalName(useHive.getState(), 2)).toBe("main on gpu-box");
  remote({ type: "command_finished", channel: 2, exit: 0, secs: 12 });
  expect(useHive.getState().inbox[0]).toMatchObject({ terminal: 2, host: HOST });
  // A pull request's offer: its host's worktree, named with its host.
  remote({
    type: "pull_ended",
    project: "/m",
    worktree: "/m",
    number: 4,
    title: "Train",
    state: "merged",
  });
  expect(useHive.getState().inbox[0]).toMatchObject({
    host: HOST,
    space: "Train · gpu-box · models",
    text: "#4 merged: remove main?",
  });
});

test("after a reattach, one summary names the agents that came to wait on the host", async () => {
  linked();
  remote(state("a", "working", false));
  remote({ type: "agent_detected", channel: 3, id: "b", project: "/m", worktree: "/m", cwd: "/m" });
  remote(state("b", "waiting_answer", true));
  shown = [];
  // The first link is no reattach; a state seen while connected changes nothing.
  link({ state: "connected" });
  link({ state: "reconnecting", reason: "gone", retry_secs: 1 });
  link({ state: "connected" });
  // Its alerts wait for the summary; `b` was waiting before the drop, `a` came to wait.
  remote(state("a", "waiting_permission", true));
  expect(shown).toEqual([]);
  await new Promise((resolve) => setTimeout(resolve, SUMMARY_MS + 50));
  expect(shown).toEqual([{ title: "1 agent waiting on gpu-box", body: "", agent: "" }]);
  expect(useHive.getState().inbox[0]).toMatchObject({ summary: true, host: HOST });
  // Done: the next alert is its own again.
  remote(state("a", "waiting_you", true));
  expect(shown).toHaveLength(2);
});

test("a summary with nothing new, or none due, says nothing", () => {
  linked();
  const s = useHive.getState();
  summarize(HOST, 0, s);
  link({ state: "reconnecting", reason: "gone", retry_secs: 1 });
  link({ state: "connected" });
  summarize(HOST, 0);
  expect(shown).toEqual([]);
  // Several at once are counted.
  link({ state: "reconnecting", reason: "gone", retry_secs: 1 });
  link({ state: "connected" });
  remote(state("a", "waiting_you", true));
  remote({ type: "agent_detected", channel: 3, id: "b", project: "/m", worktree: "/m", cwd: "/m" });
  remote(state("b", "waiting_you", true));
  summarize(HOST, 0);
  expect(shown.map((x) => x.title)).toEqual(["2 agents waiting on gpu-box"]);
  // A host removed is no reattach later.
  link({ state: "removed" });
  linked();
  notify({ type: "host_link", host: HOST, state: "connected" }, initialState);
  expect(shown).toHaveLength(1);
});

test("a remote's refusals show as toasts naming its host, shown or not (26.18 review)", () => {
  linked();
  apply({ type: "remove_project_failed", id: "/m", message: "here" });
  remote({ type: "remove_project_failed", id: "/m", message: "in use" });
  showHost(HOST);
  remote({ type: "delete_session_failed", id: "s", message: "gone" });
  const texts = useHive.getState().notices.map((n) => n.text);
  expect(texts).toEqual(["here", "gpu-box: in use", "gpu-box: Cannot delete the session: gone"]);
});
