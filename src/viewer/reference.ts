import { type HiveState, type Lines, useHive } from "../store";
import { terminal } from "../terminals";
import { transport } from "../transport";

// Selection → terminal (3.4): the viewer's selected lines become a Claude Code `@` reference
// written into the active terminal, as input with no Enter, so the user keeps typing the prompt.

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

/**
 * Where and what "Send to terminal" writes, or why it cannot: it needs selected lines and a
 * live active terminal in the open file's worktree (the one its agent is placed in by the
 * service, else the one it opened in), since the path is relative to that worktree.
 */
export function referenceTarget(
  s: HiveState,
): { terminal: number; text: string } | { why: string } {
  if (!s.openFile || !s.selectedLines) return { why: "Select lines to send their reference" };
  const tab = s.tabs.find((t) => t.id === s.activeTab);
  if (!tab) return { why: "No terminal open" };
  if (s.terminals[tab.id]?.exited) return { why: "The terminal has exited" };
  const agent = Object.values(s.agents).find((a) => a.terminal === tab.id);
  if ((agent ? agent.worktree : tab.cwd) !== s.openFile.worktree) {
    return { why: "The active terminal is in another worktree" };
  }
  return { terminal: tab.id, text: reference(s.openFile.path, s.selectedLines) };
}

/**
 * Ctrl+Shift+L and the file view's button: writes the reference and shows and focuses the
 * terminal, in front of the file's tab.
 */
export function sendReference(): void {
  const target = referenceTarget(useHive.getState());
  if ("why" in target) return;
  void transport.writeTerminal(target.terminal, target.text);
  useHive.setState({ fileShown: false, transcriptShown: null });
  terminal(target.terminal)?.focus();
}
