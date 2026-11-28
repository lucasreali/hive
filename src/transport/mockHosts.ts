import { REMOTE_TYPES } from "../hosts";
import {
  type HostLink,
  type HostMessage,
  joinedName,
  type ServiceMessage,
  type Settings,
  type Space,
} from "../protocol";
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
 * The one teammate whose Hive the fake joins (29.12): joining `alice` on any host links, and
 * serves nothing yet (29.13); anyone else's refuses the user, who is not on their list.
 */
export const MOCK_TEAMMATE = "alice";

/** The name of the `i`-th fake host after gpu-box (27.8, `&hosts=<n>`): `build-02` and on. */
export const mockHostName = (i: number) => `build-${String(i).padStart(2, "0")}`;

/**
 * The link of the `i`-th fake host (27.8): connected, but every few one reconnecting, of
 * another version, or refused, so that many hosts show every state.
 */
export function mockLink(i: number): HostLink {
  if (i % 7 === 3) {
    const reason = `Connection to ${mockHostName(i)} closed by remote host.`;
    return { state: "reconnecting", reason, retry_secs: 30 };
  }
  if (i % 11 === 5) {
    const versions = { version: "0.29.0", app_version: "0.30.0", terminals: 2 };
    return { state: "disconnected", reason: "version", kind: "version", ...versions };
  }
  if (i % 13 === 8) {
    return { state: "disconnected", reason: "Permission denied (publickey).", kind: "auth" };
  }
  return { state: "connected" };
}

/** The spaces of the `i`-th fake host: "Default", and "Work" on every third one. */
export function mockSpaces(i: number): Space[] {
  const env = { git_name: null, git_email: null, gh_account: null, claude_account: null };
  const names = i % 3 === 0 ? ["Default", "Work"] : ["Default"];
  return names.map((name) => ({ id: name.toLowerCase(), name, projects: [], env }));
}

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
  "listWorkflows",
  "runWorkflow",
  "stageFiles",
  "commit",
  "getPushTarget",
  "push",
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
  "createReview",
  "planBringBack",
  "bringBack",
  "discardReview",
  "watchWorktree",
  "unwatchWorktree",
  "expandIgnored",
  "setView",
  "listChanges",
  "markSeen",
  "listComments",
  "addComment",
  "resolveComment",
  "deleteComment",
  "listScripts",
  "listSessions",
  "locateSession",
  "deleteSession",
  "whoWrote",
  "listSessionFiles",
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
 * With `count` above 1 (27.8), fake hosts `build-02` and on up to `count`, each linked in its
 * `mockLink` state with its `mockSpaces`, serving nothing else; Reconnect or a restart links one.
 */
export async function createHostsTransport(
  cast: string | null = null,
  count = 1,
): Promise<Transport> {
  // Loaded as `index.ts` loads it: the fake service stays a chunk of its own (9.24).
  const { createMockTransport, mockProject, MOCK_OWNER } = await import("./mock");
  const repos = [mockProject(MOCK_REMOTE_REPO, "models", ["train-run"])];
  const ids = { last: 0 };
  let send: (message: HostMessage) => void = () => {};
  let up = false;
  /** The hosts the local settings list, linked or tried (26.17: `Remotes::sync`). */
  let hosts: string[] = [];
  const onLine = (line: string) => {
    if (line === "drop-link" && up) drop("reconnecting");
    if (line === "stop-link" && up) drop("disconnected");
    if (line === "no-claude") {
      // Claude Code gone from gpu-box (27.11): its service says so when asked, as now.
      remoteOptions.claude = false;
      void remote.listClaudeLogins();
    }
  };
  /** The fake hosts after gpu-box, by name, with their link and spaces. */
  const seeded = new Map<string, { link: HostLink; spaces: Space[] }>();
  for (let i = 2; i <= count; i++) {
    seeded.set(mockHostName(i), { link: mockLink(i), spaces: mockSpaces(i) });
  }
  const settings: Settings = {
    ...DEFAULT_SETTINGS,
    remote: { hosts: [MOCK_HOST, ...seeded.keys()], keep_minutes: 1440, joined: [] },
  };
  const local = createMockTransport(null, cast, { ids, settings, onLine });
  const remoteOptions = { ids, repos, onLine, claude: true, shareable: true };
  const remote = createMockTransport(null, null, remoteOptions);
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
      const fake = seeded.get(host);
      if (host === MOCK_HOST) void connectRemote(false);
      else if (fake) {
        status(host, fake.link);
        later({ type: "spaces", spaces: fake.spaces, current: "default", host });
      } else if (joined.has(host)) {
        joinHive(host, joined.get(host) as string);
      } else {
        status(host, { state: "connecting" });
        const reason = `ssh: Could not resolve hostname ${host}: nodename nor servname provided, or not known`;
        status(host, { state: "disconnected", reason, kind: "network" });
      }
    }
    hosts = next;
  };
  /** The Hives joined (29.12), by their link's name, with their owner. */
  let joined = new Map<string, string>();
  /** Links the Hive `owner` shares on `host` as `hive bridge --join` would (29.12). */
  const joinHive = (host: string, owner: string) => {
    const of = (link: HostLink) =>
      later({ type: "host_link", host, owner, ...link } as HostMessage);
    of({ state: "connecting" });
    if (owner === MOCK_TEAMMATE) return of({ state: "connected" });
    const why = `${MOCK_OWNER}'s login is not on ${owner}'s Sharing list`;
    const reason = `${owner}'s Hive on ${host.slice(owner.length + 1)} refused you: ${why}`;
    of({ state: "disconnected", reason, kind: "denied" });
  };
  const shown = () => useHive.getState().host === MOCK_HOST;
  // A fake host after gpu-box serves nothing: its requests go nowhere.
  const nowhere = async () => {};
  const hosted = Object.fromEntries(
    HOSTED.map((name) => [
      name,
      (...args: unknown[]) => {
        const host = useHive.getState().host;
        if (host !== null && seeded.has(host)) return nowhere();
        return ((shown() ? remote : local)[name] as (...args: unknown[]) => Promise<void>)(...args);
      },
    ]),
  );
  /** Links fake host `host` again (Reconnect, or a restart of its service). */
  const relink = (host: string) => {
    const fake = seeded.get(host);
    if (!fake || !hosts.includes(host) || fake.link.state === "connected") return;
    fake.link = { state: "connected" };
    status(host, fake.link);
  };
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
        if (message.type !== "settings") return;
        const remote = message.settings.remote;
        joined = new Map(remote.joined.map((j) => [joinedName(j), j.owner]));
        follow([...remote.hosts, ...joined.keys()]);
      });
    },
    async openTerminal(cwd, cols, rows, onData, account, host) {
      const remoteHost = host === undefined ? shown() : host === MOCK_HOST;
      if (!remoteHost) return local.openTerminal(cwd, cols, rows, onData, account);
      const id = await remote.openTerminal(cwd, cols, rows, onData, account);
      remoteIds.add(id);
      return id;
    },
    // This Mac's accounts, or a host's own (27.11); a fake host after gpu-box answers none.
    async listClaudeLogins(host) {
      if (host === undefined) return local.listClaudeLogins();
      if (host === MOCK_HOST) return remote.listClaudeLogins();
    },
    // A host's own Settings → Sharing (29.12): gpu-box's may be shared, this Mac's never is.
    async getSharing(host) {
      if (host === MOCK_HOST) return remote.getSharing(host);
      if (!seeded.has(host) && !joined.has(host)) return local.getSharing(host);
    },
    async setSharing(host, sharing) {
      if (host === MOCK_HOST) return remote.setSharing(host, sharing);
      if (!seeded.has(host) && !joined.has(host)) return local.setSharing(host, sharing);
    },
    async reconnectHost(host) {
      if (host === MOCK_HOST && !up && hosts.includes(host)) await connectRemote(true);
      relink(host);
    },
    async restartRemote(host) {
      relink(host);
    },
    async byeHosts(byes) {
      if (!byes.some(([host, end]) => host === MOCK_HOST && end)) return;
      for (const channel of remoteIds) await remote.closeTerminal(channel);
    },
  } as Transport;
}
