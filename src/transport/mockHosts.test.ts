import { afterEach, expect, test } from "bun:test";
import type { HostMessage } from "../protocol";
import { DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { createHostsTransport, MOCK_DROP_REASON, MOCK_HOST, MOCK_REMOTE_REPO } from "./mockHosts";

// `?mock=remote` (26.18): this Mac's fake service and gpu-box's, linked as the app side links
// a remote one.

const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

afterEach(() => useHive.setState(initialState, true));

async function linked() {
  const got: HostMessage[] = [];
  const transport = await createHostsTransport();
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
  await transport.byeHost(MOCK_HOST, false);
  await transport.byeHost("lab", true);
  await settle();
  expect(got.some((m) => m.type === "terminal_exited")).toBe(false);
  await transport.byeHost(MOCK_HOST, true);
  await settle();
  expect(got.at(-1)).toMatchObject({ type: "terminal_exited", channel: id, host: MOCK_HOST });
  const other = await transport.openTerminal(MOCK_REMOTE_REPO, 80, 24, () => {});
  got.length = 0;
  useHive.setState({ host: null });
  const settings = { ...DEFAULT_SETTINGS, remote: { hosts: ["lab"], keep_minutes: 5 } };
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
  await transport.setSettings({ ...settings, remote: { hosts: [MOCK_HOST], keep_minutes: 5 } });
  await settle();
  expect(states(got, "lab")).toEqual(["connecting", "disconnected", "removed"]);
  expect(states(got)).toEqual(["removed", "connecting", "connected"]);
});
