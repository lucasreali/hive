import { REMOTE_TYPES } from "../hosts";
import type { HostLink, HostMessage, ServiceMessage, Settings } from "../protocol";
import { DEFAULT_SETTINGS, useHive } from "../store";
import type { Transport } from ".";

// `?mock=remote` (26.18): the fake service of this Mac and a second one standing in for the
// remote host `gpu-box`, linked as the app side links one (26.17): its messages carry `host`,
// and only those the app side passes reach the UI (`REMOTE_TYPES`).

/** The fake remote host's alias. */
export const MOCK_HOST = "gpu-box";

/** The remote host's repository, `models`, followed from the start in its "Default" space. */
export const MOCK_REMOTE_REPO = "/home/dev/models";

/** Why the fake link dropped (`drop-link` typed in a terminal). */
export const MOCK_DROP_REASON = "Connection to gpu-box closed by remote host.";

/**
 * The requests that go to the shown host's service (`HiveState.host`), as the app side routes
 * them by their `host` (`src/transport/tauri.ts`); every other one is this Mac's.
 */
const HOSTED = [
  "listProjects",
  "addProject",
  "removeProject",
  "initGit",
  "createProject",
  "createDir",
  "createSpace",
  "updateSpace",
  "deleteSpace",
  "selectSpace",
  "listGhAccounts",
  "switchGhAccount",
  "getGhIdentity",
  "listPulls",
  "openPull",
  "actOnPull",
  "createPull",
  "listRuns",
  "openRun",
  "openJobLog",
  "actOnRun",
  "listDirs",
  "listBranches",
  "validateWorktreeName",
  "createWorktree",
  "createIssueWorktree",
  "removeWorktree",
  "renameWorktree",
  "watchWorktree",
  "unwatchWorktree",
  "expandIgnored",
  "setView",
  "listChanges",
  "listScripts",
  "listSessions",
  "locateSession",
  "deleteSession",
  "moveSession",
  "searchFiles",
  "openFile",
  "openTable",
  "editTable",
  "saveFile",
  "createFile",
  "renameFile",
  "moveFile",
  "deleteFile",
  "createFolder",
  "openInEditor",
] as const satisfies readonly (keyof Transport)[];

/** A terminal's requests: they go to the service that opened it. */
const TERMINAL = [
  "attachTerminal",
  "writeTerminal",
  "resizeTerminal",
  "closeTerminal",
  "ackTerminal",
] as const satisfies readonly (keyof Transport)[];

/**
 * This Mac's fake service and `gpu-box`'s. Typed in any terminal, `drop-link` drops the link
 * (`reconnecting`, until Reconnect) and `stop-link` stops it (`disconnected`, a network failure).
 */
export async function createHostsTransport(cast: string | null = null): Promise<Transport> {
  // Loaded as `index.ts` loads it: the fake service stays a chunk of its own (9.24).
  const { createMockTransport, mockProject } = await import("./mock");
  const repos = [mockProject(MOCK_REMOTE_REPO, "models", ["train-run"])];
  const ids = { last: 0 };
  let send: (message: HostMessage) => void = () => {};
  let up = false;
  /** The hosts the local settings list, linked or tried (26.17: `Remotes::sync`). */
  let hosts: string[] = [];
  const onLine = (line: string) => {
    if (line === "drop-link" && up) drop("reconnecting");
    if (line === "stop-link" && up) drop("disconnected");
  };
  const settings: Settings = {
    ...DEFAULT_SETTINGS,
    remote: { hosts: [MOCK_HOST], keep_minutes: 1440 },
  };
  const local = createMockTransport(null, cast, { ids, settings, onLine });
  const remote = createMockTransport(null, null, { ids, repos, onLine });
  /** The remote's terminals, by id. */
  const remoteIds = new Set<number>();
  const later = (message: HostMessage) => setTimeout(() => send(message), 0);
  const status = (host: string, link: HostLink) =>
    later({ type: "host_link", host, ...link } as HostMessage);
  const fromRemote = (message: ServiceMessage) => {
    if (!up) return;
    if (message.type === "welcome") return status(MOCK_HOST, { state: "connected" });
    if (message.type === "terminal_exited") remoteIds.delete(message.channel);
    if ((REMOTE_TYPES as readonly string[]).includes(message.type)) {
      send({ ...message, host: MOCK_HOST });
    }
  };
  const connectRemote = async (again: boolean) => {
    up = true;
    status(MOCK_HOST, { state: "connecting" });
    await remote.connect(fromRemote);
    // Its kept terminals are attached again (26.16) and redrawn.
    if (again) {
      for (const channel of remoteIds)
        later({ type: "terminal_reattached", channel, host: MOCK_HOST });
    }
  };
  const drop = (state: "reconnecting" | "disconnected") => {
    up = false;
    status(
      MOCK_HOST,
      state === "reconnecting"
        ? { state, reason: MOCK_DROP_REASON, retry_secs: 30 }
        : { state, reason: MOCK_DROP_REASON, kind: "network" },
    );
  };
  /** Follows the hosts of the local settings: a new one is linked, one gone is removed. */
  const follow = (next: string[]) => {
    for (const host of hosts.filter((h) => !next.includes(h))) {
      if (host === MOCK_HOST) {
        up = false;
        for (const channel of remoteIds) {
          later({ type: "terminal_exited", channel, code: null, host: MOCK_HOST });
        }
        remoteIds.clear();
      }
      status(host, { state: "removed" });
    }
    for (const host of next.filter((h) => !hosts.includes(h))) {
      if (host === MOCK_HOST) void connectRemote(false);
      else {
        status(host, { state: "connecting" });
        const reason = `ssh: Could not resolve hostname ${host}: nodename nor servname provided, or not known`;
        status(host, { state: "disconnected", reason, kind: "network" });
      }
    }
    hosts = next;
  };
  const shown = () => useHive.getState().host === MOCK_HOST;
  const hosted = Object.fromEntries(
    HOSTED.map((name) => [
      name,
      (...args: unknown[]) =>
        ((shown() ? remote : local)[name] as (...args: unknown[]) => Promise<void>)(...args),
    ]),
  );
  const terminal = Object.fromEntries(
    TERMINAL.map((name) => [
      name,
      (id: number, ...args: unknown[]) =>
        ((remoteIds.has(id) ? remote : local)[name] as (...args: unknown[]) => Promise<void>)(
          id,
          ...args,
        ),
    ]),
  );
  return {
    ...local,
    ...hosted,
    ...terminal,
    async connect(onMessage) {
      send = onMessage;
      await local.connect((message) => {
        onMessage(message);
        if (message.type === "settings") follow(message.settings.remote.hosts);
      });
    },
    async openTerminal(cwd, cols, rows, onData, account, host) {
      const remoteHost = host === undefined ? shown() : host === MOCK_HOST;
      if (!remoteHost) return local.openTerminal(cwd, cols, rows, onData, account);
      const id = await remote.openTerminal(cwd, cols, rows, onData, account);
      remoteIds.add(id);
      return id;
    },
    async reconnectHost(host) {
      if (host === MOCK_HOST && !up && hosts.includes(host)) await connectRemote(true);
    },
    async restartRemote() {},
    async byeHosts(byes) {
      if (!byes.some(([host, end]) => host === MOCK_HOST && end)) return;
      for (const channel of remoteIds) await remote.closeTerminal(channel);
    },
  } as Transport;
}
