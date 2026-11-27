import { safeStorage } from "../persist";
import type { Agent } from "../protocol";
import { findWorktree, type HiveState, inAgentOrder } from "../store";
import { onShownHost } from "../tabs";

// Where review comments and line references go (28.21): to an agent of the worktree, the one
// the user picks, never a plain terminal (a dev server, a shell) that happens to be active.

/** Why nothing can be sent: the worktree has no live agent. */
export const NO_AGENT = "No agent in this worktree";

/**
 * The live agents of `worktree`, in the user's order (8.2): each one the service placed there
 * whose terminal is open on the shown host and still running. A terminal no agent runs in is
 * never one of them.
 */
export function agentsIn(s: HiveState, worktree: string): Agent[] {
  const live = (a: Agent) =>
    a.worktree === worktree &&
    s.tabs.some((t) => t.id === a.terminal && onShownHost(s, t)) &&
    !s.terminals[a.terminal]?.exited &&
    s.agentStates[a.id]?.state !== "ended";
  return inAgentOrder(Object.values(s.agents).filter(live), s.agentOrder);
}

/** What an agent's entry in the picker is called: its tab's title. */
export const agentLabel = (s: HiveState, a: Agent): string =>
  s.agentTitles[a.id] ?? findWorktree(s.projects, a.worktree)?.name ?? "Claude";

/** Where the agent last picked in each worktree is remembered (UI state, #37). */
const STORAGE = "hive.sendTarget";
/** How many worktrees' picks are remembered; the oldest are forgotten first. */
export const PICK_LIMIT = 200;

type Store = Pick<Storage, "getItem" | "setItem"> | null;

function saved(storage: Store): [string, string][] {
  const read: unknown = JSON.parse(storage?.getItem(STORAGE) ?? "{}");
  if (!read || typeof read !== "object" || Array.isArray(read)) return [];
  return Object.entries(read).filter((e): e is [string, string] => typeof e[1] === "string");
}

/** The session id last picked in `worktree`; null when none or the storage is unreadable. */
export function savedPick(worktree: string, storage: Store = safeStorage()): string | null {
  try {
    return saved(storage).find(([w]) => w === worktree)?.[1] ?? null;
  } catch {
    return null;
  }
}

/** Remembers `id` as the agent picked in `worktree`. */
export function savePick(worktree: string, id: string, storage: Store = safeStorage()): void {
  try {
    const kept = saved(storage).filter(([w]) => w !== worktree);
    const picks = [...kept, [worktree, id]].slice(-PICK_LIMIT);
    storage?.setItem(STORAGE, JSON.stringify(Object.fromEntries(picks)));
  } catch {
    // A full or blocked storage only loses the preference.
  }
}

/**
 * The agents of `worktree` and the one a send without a menu goes to (the palette, Ctrl+Shift+L):
 * the one last picked there while it lives, else the first; or why there is none.
 */
export function agentTarget(
  s: HiveState,
  worktree: string,
  storage: Store = safeStorage(),
): { agents: Agent[]; agent: Agent } | { why: string } {
  const agents = agentsIn(s, worktree);
  const [first] = agents;
  if (!first) return { why: NO_AGENT };
  const picked = agents.length > 1 ? savedPick(worktree, storage) : null;
  return { agents, agent: agents.find((a) => a.id === picked) ?? first };
}

/**
 * The terminal a send to `worktree` writes into: `chosen` (a terminal picked in the menu) while
 * its agent lives there, else the target agent's; or why it cannot.
 */
export function terminalIn(
  s: HiveState,
  worktree: string,
  chosen?: number,
  storage: Store = safeStorage(),
): { terminal: number } | { why: string } {
  const target = agentTarget(s, worktree, storage);
  if ("why" in target) return target;
  if (chosen === undefined) return { terminal: target.agent.terminal };
  if (target.agents.some((a) => a.terminal === chosen)) return { terminal: chosen };
  return { why: "That agent has ended" };
}
