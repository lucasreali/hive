import { afterEach, expect, test } from "bun:test";
import type { HostMessage } from "../protocol";
import { DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { MOCK_OWNER, MOCK_UNSHAREABLE, mockSharingError } from "./mock";
import {
  createHostsTransport,
  MOCK_DROP_REASON,
  MOCK_HOST,
  MOCK_REMOTE_REPO,
  MOCK_TEAMMATE,
  mockHostName,
} from "./mockHosts";

// `?mock=remote` (26.18): this Mac's fake service and gpu-box's, linked as the app side links
// a remote one.

const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

afterEach(() => useHive.setState(initialState, true));

async function linked(count = 1) {
  const got: HostMessage[] = [];
  const transport = await createHostsTransport(null, count);
  await transport.connect((m) => got.push(m));
  await settle();
  return { transport, got };
}

const states = (got: HostMessage[], host = MOCK_HOST) =>
  got.flatMap((m) => (m.type === "host_link" && m.host === host ? [m.state] : []));

test("this Mac's service links to gpu-box, whose messages carry its host", async () => {
  const { transport, got } = await linked();
  expect(states(got)).toEqual(["connecting", "connected"]);
  const projects = got.filter((m) => m.type === "projects");
  expect(projects.map((m) => m.host)).toEqual([undefined, MOCK_HOST]);
  const remote = projects[1] as Extract<HostMessage, { type: "projects" }>;
  expect(remote.projects.map((p) => p.id)).toEqual([MOCK_REMOTE_REPO]);
  // Only what the app side passes: never the remote's welcome or settings.
  const fromRemote = got.filter((m) => m.host === MOCK_HOST).map((m) => m.type);
  expect(fromRemote).not.toContain("settings");
  expect(fromRemote).not.toContain("welcome");
  // The shown host's requests go to its service.
  got.length = 0;
  useHive.setState({ host: MOCK_HOST });
  await transport.listProjects();
  await settle();
  expect(got.find((m) => m.type === "projects")?.host).toBe(MOCK_HOST);
});

test("a remote terminal drops with its link, takes Reconnect and is redrawn", async () => {
  const { transport, got } = await linked();
  useHive.setState({ host: MOCK_HOST });
  const remote = await transport.openTerminal(MOCK_REMOTE_REPO, 80, 24, () => {});
  useHive.setState({ host: null });
  const local = await transport.openTerminal("/home/user/projects/shop", 80, 24, () => {});
  expect(local).toBe(remote + 1);
  // Its requests go to the remote's service whatever host shows.
  await transport.writeTerminal(remote, "drop-link\r");
  await settle();
  expect(got.at(-1)).toEqual({
    type: "host_link",
    host: MOCK_HOST,
    state: "reconnecting",
    reason: MOCK_DROP_REASON,
    retry_secs: 30,
  });
  // Down: nothing of the remote's comes; this Mac's own Reconnect only once down.
  got.length = 0;
  await transport.writeTerminal(remote, "exit\r");
  await transport.reconnectHost("lab");
  await settle();
  expect(got).toEqual([]);
  await transport.reconnectHost(MOCK_HOST);
  await settle();
  expect(states(got)).toEqual(["connecting", "connected"]);
  await transport.reconnectHost(MOCK_HOST);
  await settle();
  expect(states(got)).toEqual(["connecting", "connected"]);
  // `stop-link` stops it: a network failure, until Reconnect.
  await transport.writeTerminal(local, "stop-link\r");
  await settle();
  expect(got.at(-1)).toMatchObject({ state: "disconnected", kind: "network" });
  await transport.restartRemote(MOCK_HOST);
});

test("a kept terminal is reattached after a reconnect", async () => {
  const { transport, got } = await linked();
  useHive.setState({ host: MOCK_HOST });
  const id = await transport.openTerminal(MOCK_REMOTE_REPO, 80, 24, () => {});
  await transport.writeTerminal(id, "drop-link\r");
  await transport.reconnectHost(MOCK_HOST);
  await settle();
  expect(got).toContainEqual({ type: "terminal_reattached", channel: id, host: MOCK_HOST });
});

test("the hosts follow this Mac's settings; quitting may end a host's terminals", async () => {
  const { transport, got } = await linked();
  useHive.setState({ host: MOCK_HOST });
  const id = await transport.openTerminal(MOCK_REMOTE_REPO, 80, 24, () => {});
  await transport.byeHosts([[MOCK_HOST, false]]);
  await transport.byeHosts([["lab", true]]);
  await settle();
  expect(got.some((m) => m.type === "terminal_exited")).toBe(false);
  await transport.byeHosts([
    ["lab", false],
    [MOCK_HOST, true],
  ]);
  await settle();
  expect(got.at(-1)).toMatchObject({ type: "terminal_exited", channel: id, host: MOCK_HOST });
  const other = await transport.openTerminal(MOCK_REMOTE_REPO, 80, 24, () => {});
  got.length = 0;
  useHive.setState({ host: null });
  const settings = { ...DEFAULT_SETTINGS, remote: { hosts: ["lab"], keep_minutes: 5, joined: [] } };
  await transport.setSettings(settings);
  await settle();
  expect(got).toContainEqual({
    type: "terminal_exited",
    channel: other,
    code: null,
    host: MOCK_HOST,
  });
  expect(states(got)).toEqual(["removed"]);
  expect(states(got, "lab")).toEqual(["connecting", "disconnected"]);
  // gpu-box again: linked again.
  await transport.setSettings({
    ...settings,
    remote: { hosts: [MOCK_HOST], keep_minutes: 5, joined: [] },
  });
  await settle();
  expect(states(got, "lab")).toEqual(["connecting", "disconnected", "removed"]);
  expect(states(got)).toEqual(["removed", "connecting", "connected"]);
});

test("with a host count, fake hosts come in every state with their spaces (27.8)", async () => {
  const { transport, got } = await linked(14);
  const host = (i: number) => mockHostName(i);
  expect(host(2)).toBe("build-02");
  expect(states(got, host(2))).toEqual(["connected"]);
  expect(got).toContainEqual({
    type: "host_link",
    host: host(3),
    state: "reconnecting",
    reason: "Connection to build-03 closed by remote host.",
    retry_secs: 30,
  });
  expect(got).toContainEqual({
    type: "host_link",
    host: host(5),
    state: "disconnected",
    reason: "version",
    kind: "version",
    version: "0.29.0",
    app_version: "0.30.0",
    terminals: 2,
  });
  expect(got).toContainEqual({
    type: "host_link",
    host: host(8),
    state: "disconnected",
    reason: "Permission denied (publickey).",
    kind: "auth",
  });
  const spaces = (i: number) =>
    got.flatMap((m) =>
      m.type === "spaces" && m.host === host(i) ? [m.spaces.map((s) => s.name)] : [],
    );
  expect(spaces(2)).toEqual([["Default"]]);
  expect(spaces(3)).toEqual([["Default", "Work"]]);
  // A fake host serves nothing.
  got.length = 0;
  useHive.setState({ host: host(2) });
  await transport.listProjects();
  await settle();
  expect(got).toEqual([]);
  // Reconnect or a restart links one; a linked one stays as it is.
  await transport.reconnectHost(host(3));
  await transport.restartRemote(host(5));
  await transport.reconnectHost(host(2));
  await settle();
  expect(got.map((m) => m.type === "host_link" && [m.host, m.state])).toEqual([
    [host(3), "connected"],
    [host(5), "connected"],
  ]);
  // Not once it is removed.
  got.length = 0;
  useHive.setState({ host: null });
  await transport.setSettings({
    ...DEFAULT_SETTINGS,
    remote: { hosts: [], keep_minutes: 5, joined: [] },
  });
  await settle();
  await transport.reconnectHost(host(8));
  await settle();
  expect(states(got, host(8))).toEqual(["removed"]);
});

test("each host answers its own Claude accounts; `no-claude` takes claude off gpu-box (27.11)", async () => {
  const { transport, got } = await linked(2);
  const logins = () => got.filter((m) => m.type === "claude_logins");
  got.length = 0;
  await transport.listClaudeLogins();
  await transport.listClaudeLogins(MOCK_HOST);
  // A fake host after gpu-box answers nothing.
  await transport.listClaudeLogins(mockHostName(2));
  await settle();
  expect(logins().map((m) => [m.host, m.claude])).toEqual([
    [undefined, true],
    [MOCK_HOST, true],
  ]);
  got.length = 0;
  const id = await transport.openTerminal("/home/user/projects/shop", 80, 24, () => {});
  await transport.writeTerminal(id, "no-claude\r");
  await settle();
  expect(logins().map((m) => [m.host, m.claude])).toEqual([[MOCK_HOST, false]]);
});

test("a Hive joined links by its owner, and only gpu-box's own Sharing may be set (29.12)", async () => {
  const { transport, got } = await linked(3);
  const joined = [
    { alias: MOCK_HOST, owner: MOCK_TEAMMATE },
    { alias: MOCK_HOST, owner: "carol" },
  ];
  const remote = { hosts: [MOCK_HOST, mockHostName(2)], keep_minutes: 5, joined };
  await transport.setSettings({ ...DEFAULT_SETTINGS, remote });
  await settle();
  const alice = `${MOCK_TEAMMATE}@${MOCK_HOST}`;
  expect(states(got, alice)).toEqual(["connecting", "connected"]);
  expect(got.find((m) => m.type === "host_link" && m.host === alice)).toMatchObject({
    owner: MOCK_TEAMMATE,
  });
  const refused = got.filter((m) => m.type === "host_link" && m.host === `carol@${MOCK_HOST}`);
  expect(refused.at(-1)).toMatchObject({
    state: "disconnected",
    kind: "denied",
    owner: "carol",
    reason: `carol's Hive on ${MOCK_HOST} refused you: ${MOCK_OWNER}'s login is not on carol's Sharing list`,
  });
  // gpu-box's own Sharing: it may be shared; a fake host, or a joined one, answers nothing.
  got.length = 0;
  await transport.getSharing(MOCK_HOST);
  await transport.getSharing(mockHostName(2));
  await transport.getSharing(alice);
  await settle();
  const answers = got.filter((m) => m.type === "sharing");
  expect(answers).toHaveLength(1);
  expect(answers[0]).toMatchObject({ host: MOCK_HOST, owner: MOCK_OWNER, unavailable: null });
  got.length = 0;
  const on = { on: true, group: "team", spaces: ["default"], people: [] };
  await transport.setSharing(MOCK_HOST, on);
  await transport.setSharing(mockHostName(2), on);
  await transport.setSharing(alice, on);
  await transport.setSharing(MOCK_HOST, { ...on, group: "" });
  await settle();
  expect(got.filter((m) => m.type === "sharing")).toHaveLength(1);
  expect(got.find((m) => m.type === "sharing")).toMatchObject({ host: MOCK_HOST, sharing: on });
  expect(got.find((m) => m.type === "sharing_failed")).toMatchObject({
    host: MOCK_HOST,
    message: "Pick the Unix group your teammates are in",
  });
  // This Mac's own is never shared.
  got.length = 0;
  await transport.getSharing("this-mac");
  await transport.setSharing("this-mac", on);
  await settle();
  expect(got.find((m) => m.type === "sharing")).toMatchObject({ unavailable: MOCK_UNSHAREABLE });
  expect(got.find((m) => m.type === "sharing_failed")).toMatchObject({
    message: MOCK_UNSHAREABLE,
  });
});

test("the fake service refuses a Sharing list as the real one would (29.12)", () => {
  const person = (login: string) => ({
    login,
    display: "",
    git_name: "",
    git_email: "",
    role: "viewer" as const,
  });
  const ok = { on: true, group: "team", spaces: [], people: [person("bob")] };
  expect(mockSharingError(ok)).toBeNull();
  expect(mockSharingError({ ...ok, on: false, group: "" })).toBeNull();
  expect(mockSharingError({ ...ok, group: "wheel" })).toBe('You are not in the group "wheel"');
  expect(mockSharingError({ ...ok, people: [person("Bob")] })).toBe(
    'The login name "Bob" may hold only a-z, 0-9, _ and -, and starts with a letter or _',
  );
  expect(mockSharingError({ ...ok, people: [person(MOCK_OWNER)] })).toBe(
    `${MOCK_OWNER} is you: this Hive is yours`,
  );
});
