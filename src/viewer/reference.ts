import { activateTab, type HiveState, type Lines, useHive } from "../store";
import { showTerminal } from "../terminals";
import { transport } from "../transport";
import { terminalIn } from "./target";

// Selection → agent (3.4, 28.21): the viewer's selected lines become a Claude Code `@` reference
// written into an agent's terminal of that worktree, as input with no Enter, so the user keeps
// typing the prompt.

/**
 * `@src/a.ts (lines 44–46) ` or `@src/a.ts (line 44) `, with a trailing space. A path with
 * whitespace is double-quoted (`@"docs/my notes.md"`), as Claude Code's own `@` completion
 * does; the spike (1.12) confirms it.
 */
export function reference(path: string, { from, to }: Lines): string {
  const at = /\s/.test(path) ? `"${path}"` : path;
  const lines = from === to ? `line ${from}` : `lines ${from}–${to}`;
  return `@${at} (${lines}) `;
}

const NO_SELECTION = "Select lines to send their reference";

/** Why the shown file has no reference to send (no lines selected), or null. */
export const noSelection = (s: Pick<HiveState, "openFile" | "selectedLines">) =>
  s.openFile && s.selectedLines ? null : NO_SELECTION;

/**
 * Where and what "Send to agent" writes, or why it cannot: it needs selected lines and an agent
 * of the file's worktree (`chosen`, a terminal picked in the menu, else the target, 28.21).
 */
export function referenceTarget(
  s: HiveState,
  chosen?: number,
): { terminal: number; text: string } | { why: string } {
  const { openFile, selectedLines } = s;
  if (!openFile || !selectedLines) return { why: NO_SELECTION };
  const target = terminalIn(s, openFile.worktree, chosen);
  if ("why" in target) return target;
  return { ...target, text: reference(openFile.path, selectedLines) };
}

/** Shows the tab of terminal `id`, in front of a file's tab, and gives it the keyboard. */
export function showAgentTerminal(id: number): void {
  const tab = useHive.getState().tabs.find((t) => t.id === id);
  if (tab) activateTab(tab);
  showTerminal(id);
}

/**
 * Ctrl+Shift+L and the file view's button: writes the reference into the agent's terminal and
 * shows and focuses it, in front of the file's tab.
 */
export function sendReference(chosen?: number): void {
  const target = referenceTarget(useHive.getState(), chosen);
  if ("why" in target) return;
  void transport.writeTerminal(target.terminal, target.text);
  showAgentTerminal(target.terminal);
}
