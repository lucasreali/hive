import type { Script, WorktreeScripts } from "./protocol";
import { type HiveState, scriptsOf, useHive } from "./store";
import { openWith } from "./terminals";
import { transport } from "./transport";

// A worktree's scripts (23.4): the service lists them (the project's run scripts from the
// settings first, then the repository's); the app only shows them, types the picked one into a
// new terminal there, and remembers which terminal runs which script (UI state).

/** A script started from Hive, still running as far as the app knows. */
export type RunningScript = { id: number; name: string; command: string };

/**
 * The scripts of the worktree `path` of the project `project`: the service's list once it sent
 * one, until then the project's run scripts from the settings. Pure: select its inputs with
 * stable values (`s.scripts[path]`, `s.settings`) and call it outside the selector.
 */
export function worktreeScripts(
  listed: WorktreeScripts | undefined,
  settings: HiveState["settings"],
  project: string | undefined,
): Script[] {
  if (listed) return listed.scripts;
  const run = scriptsOf(settings, project).run;
  return run.map((r) => ({ ...r, source: "settings" }));
}

/** Types `script` into a new terminal in the worktree `path` and remembers that it runs there. */
export async function runScript(path: string, script: Script): Promise<number> {
  const id = await openWith(path, script.command);
  const run = { worktree: path, name: script.name, command: script.command };
  useHive.setState((s) => ({ scriptRuns: { ...s.scriptRuns, [id]: run } }));
  return id;
}

/** Stops the script running in terminal `id` as a user would, with Ctrl+C; its tab stays. */
export function stopScript(id: number): void {
  void transport.writeTerminal(id, "\x03");
  useHive.setState((s) => {
    const { [id]: _, ...rest } = s.scriptRuns;
    return { scriptRuns: rest };
  });
}

/**
 * The scripts started from Hive in the worktree `path` that still run: their terminal is open
 * and has not exited, and they were not stopped here. One that ends on its own stays listed
 * until its terminal closes (Hive does not watch the command itself).
 */
export function runningScripts(
  s: Pick<HiveState, "scriptRuns" | "tabs" | "terminals">,
  path: string,
): RunningScript[] {
  return Object.entries(s.scriptRuns)
    .map(([id, run]) => ({ id: Number(id), ...run }))
    .filter((r) => r.worktree === path)
    .filter((r) => s.tabs.some((t) => t.id === r.id) && !s.terminals[r.id]?.exited)
    .map(({ id, name, command }) => ({ id, name, command }));
}
