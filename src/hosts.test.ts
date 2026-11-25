import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import samples from "../crates/hive-protocol/tests/app-messages.json";
import { onMessage } from "./connect";
import { agentHost, agentKey, keyed, REMOTE_TYPES, spaceLabel, taken } from "./hosts";
import type { HostMessage } from "./protocol";
import { apply } from "./reduce";
import {
  addTab,
  DEFAULT_SETTINGS,
  everyHost,
  hostData,
  hostView,
  initialState,
  knownHosts,
  pendingAgents,
  showHost,
  useHive,
} from "./store";
import { tabWorktree, visibleTabs } from "./tabs";
import { transport } from "./transport";

// 26.18: a remote host's messages are applied to that host's data only, and only those the UI
// takes of a remote (`REMOTE_TYPES`, the app side's `REMOTE_TO_UI`).

const HOST = "gpu-box";

/** A sample of `type`, as the pump forwards it: on channel 1. */
function sample(type: string): HostMessage {
  const found = samples.find((m) => m.type === type);
  if (!found) throw new Error(`no sample of ${type}`);
  return { ...found, channel: 1 } as unknown as HostMessage;
}

/** This Mac's data from every sample the UI takes of a remote (muted: no Web Audio here). */
function localData() {
  useHive.setState({
    settings: {
      ...DEFAULT_SETTINGS,
      notifications: { ...DEFAULT_SETTINGS.notifications, volume: 0 },
    },
  });
  for (const type of REMOTE_TYPES) onMessage(sample(type));
  return hostData(useHive.getState());
}

const reset = () => useHive.setState(initialState, true);
beforeEach(reset);
afterEach(reset);

test("the UI takes of a remote exactly what the app side passes it (REMOTE_TO_UI)", async () => {
  const lib = await Bun.file(`${import.meta.dir}/../src-tauri/src/lib.rs`).text();
  const list = /const REMOTE_TO_UI: &\[&str\] = &\[([^\]]*)\];/.exec(lib)?.[1] ?? "";
  const rust = [...list.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  expect([...REMOTE_TYPES].sort() as string[]).toEqual(rust.sort());
});

for (const type of REMOTE_TYPES) {
  test(`a remote's ${type} is applied to its host's data, never this Mac's`, () => {
    const before = localData();
    // This Mac shown: its data stays as it was, even where the remote names its ids (`/r`).
    onMessage({ ...sample(type), host: HOST });
    expect(hostData(useHive.getState())).toEqual(before);
    // The host shown: this Mac's data, kept aside, stays as it was too.
    showHost(HOST);
    onMessage({ ...sample(type), host: HOST });
    expect(useHive.getState().stash[""]).toEqual(before as never);
    // And this Mac's own message leaves the host's data shown alone.
    const remote = hostData(useHive.getState());
    onMessage(sample(type));
    expect(hostData(useHive.getState())).toEqual(remote);
  });
}

test("a remote's projects, spaces and agents are its own, keyed apart from this Mac's", () => {
  localData();
  // Its samples end with the agent removed: this Mac's comes back.
  onMessage(sample("agent_detected"));
  for (const type of ["spaces", "projects", "agent_detected", "agent_state"]) {
    onMessage({ ...sample(type), host: HOST });
  }
  const s = useHive.getState();
  const remote = hostView(s, HOST);
  expect(Object.keys(remote.projects ?? {})).toEqual(["/r"]);
  expect(remote.spaces?.map((x) => x.id)).toEqual(["default"]);
  // The same session id on two hosts is two agents.
  expect(Object.keys(s.agents)).toEqual(["s"]);
  expect(Object.keys(remote.agents)).toEqual([agentKey(HOST, "s")]);
  expect(remote.agents[agentKey(HOST, "s")]?.id).toBe(`${HOST} s`);
  expect(Object.keys(everyHost(s).agents).sort()).toEqual(["gpu-box s", "s"]);
  expect(knownHosts(s)).toEqual([null, HOST]);
  // The local service's view is the state itself.
  expect(hostView(s, null)).toBe(s);
});

test("only REMOTE_TYPES and the link's own messages are taken of a remote", () => {
  for (const type of ["settings", "restore_sessions", "editor_target", "session_located"]) {
    expect(taken({ ...sample(type), host: HOST })).toBe(false);
    expect(taken(sample(type))).toBe(true);
  }
  const link = { type: "host_link", host: HOST, state: "connected" } as HostMessage;
  expect(taken(link)).toBe(true);
  expect(taken({ type: "terminal_reattached", channel: 2, host: HOST })).toBe(true);
  // Nothing of one it does not take reaches the store.
  onMessage({ ...sample("settings"), host: HOST });
  onMessage({ type: "quit_requested", host: HOST });
  expect(useHive.getState().settings).toEqual(initialState.settings);
  expect(useHive.getState().modal).toBeNull();
});

test("agent keys name their host; a local agent keeps its id", () => {
  expect(agentKey(null, "s")).toBe("s");
  expect(agentKey(HOST, "s")).toBe("gpu-box s");
  expect(agentHost("s")).toBeNull();
  expect(agentHost("gpu-box s")).toBe(HOST);
  // An id with a space of its own still names its host by the first.
  expect(agentHost(agentKey("me@box", "a b"))).toBe("me@box");
  const state = sample("agent_state");
  expect(keyed(state)).toBe(state);
  expect(keyed({ ...state, host: HOST })).toMatchObject({ id: "gpu-box s", host: HOST });
  const opened = { ...sample("terminal_opened"), host: HOST };
  expect(keyed(opened)).toBe(opened);
  expect(spaceLabel({ name: "Work" }, HOST)).toBe("Work · gpu-box");
  expect(spaceLabel({ name: "Work" }, null)).toBe("Work");
  expect(spaceLabel(undefined, HOST)).toBe(HOST);
});

test("a remote terminal naming a local worktree never shows in it", () => {
  apply({ ...sample("projects") });
  useHive.setState({ selection: "/r" });
  // This Mac's terminal in /r, and the remote's that names /r too.
  addTab(1, "/r");
  apply({ type: "terminal_opened", channel: 1, worktree: "/r", account: "Work" });
  addTab(2, "/r", HOST);
  apply({ type: "terminal_opened", channel: 2, worktree: "/r", account: "Work", host: HOST });
  const s = useHive.getState();
  expect(s.terminals[2]?.host).toBe(HOST);
  expect(s.activeTab).toBe(1);
  expect(visibleTabs(s).map((t) => t.id)).toEqual([1]);
  expect(tabWorktree(s, s.tabs[1] as never)).not.toBe("/r");
  // Its own host shows it there, and not this Mac's.
  showHost(HOST);
  const remote = useHive.getState();
  expect(visibleTabs({ ...remote, selection: "/r" }).map((t) => t.id)).toEqual([2]);
  expect(visibleTabs({ ...remote, selection: null }).map((t) => t.id)).toEqual([2]);
});

test("a remote's projects never move this Mac's terminals", () => {
  apply(sample("projects"));
  addTab(1, "/r");
  apply({ type: "terminal_opened", channel: 1, worktree: "/r", account: "Work" });
  // The host's list without /r: this Mac's terminal stays in /r.
  apply({ type: "projects", projects: [], host: HOST });
  showHost(HOST);
  apply({ type: "projects", projects: [], host: HOST });
  expect(useHive.getState().terminals[1]?.worktree).toBe("/r");
});

test("the bell and F8 count every host's pending agents", () => {
  const pending = { ...sample("agent_state"), pending: true, state: "waiting_permission" };
  apply(sample("agent_detected"));
  apply(pending as HostMessage);
  onMessage({ ...sample("agent_detected"), host: HOST });
  onMessage({ ...pending, host: HOST } as HostMessage);
  expect(pendingAgents(useHive.getState()).map((a) => a.id)).toEqual(["s", "gpu-box s"]);
});

test("a remote terminal reattached with a local worktree's path as its cwd stays on its host", async () => {
  // The service kept it while the app was gone (26.16); its host has the same folder layout.
  const attach = spyOn(transport, "attachTerminal").mockResolvedValue();
  const resize = spyOn(transport, "resizeTerminal").mockResolvedValue();
  apply(sample("projects"));
  useHive.setState({ selection: "/r" });
  onMessage({
    type: "terminal_kept",
    channel: 7,
    cwd: "/r",
    worktree: null,
    account: null,
    host: HOST,
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(attach).toHaveBeenCalledWith(7, expect.any(Function));
  const s = useHive.getState();
  expect(s.tabs).toEqual([{ id: 7, cwd: "/r", host: HOST }]);
  expect(s.terminals[7]?.host).toBe(HOST);
  // Not in this Mac's /r, nor shown over it.
  expect(visibleTabs(s)).toEqual([]);
  expect(s.activeTab).toBeNull();
  showHost(HOST);
  expect(visibleTabs({ ...useHive.getState(), selection: "/r" }).map((t) => t.id)).toEqual([7]);
  attach.mockRestore();
  resize.mockRestore();
});
