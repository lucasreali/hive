import { FileTextIcon, GitBranchIcon, type Icon, LightningIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { COMMANDS, currentProject, goToAgent } from "../shortcuts";
import {
  type HiveState,
  openModal,
  panelWorktree,
  scriptsOf,
  select,
  spaceProjects,
  treeAgents,
  useHive,
} from "../store";
import { openWith } from "../terminals";
import { transport } from "../transport";
import { reviewTarget, sendReview } from "../viewer/review";
import { keyText } from "../window";
import { ICON, SearchIcon, STATE_LABEL } from "./icons";
import { leaveFile, SEARCH_DELAY_MS } from "./RightPanel";

// The command palette (Ctrl+Shift+P, 6.3): one filtered list of commands, agents and worktrees,
// and the lines of the selected worktree's files holding the typed text.

/** A palette row; `keys` is the shortcut hint of a command. */
export type PaletteItem = { label: string; detail?: string; keys?: string; run: () => void };
type Group = { name: string; icon: Icon; items: PaletteItem[] };

/** Most file matches listed: the rest is the Files panel's job. */
export const FILE_LIMIT = 50;

/** Characters after which a match starts a word. */
const WORD_START = " /-_.:";

/**
 * How well `query` matches `text` as a subsequence, ignoring case (higher is better), or null
 * when it does not. Consecutive characters and word starts score more; an empty query scores 0.
 */
export function fuzzy(query: string, text: string): number | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  let score = 0;
  let at = 0;
  let last = -2;
  for (const c of q) {
    const i = t.indexOf(c, at);
    if (i < 0) return null;
    score +=
      1 + (i === last + 1 ? 4 : 0) + (i === 0 || WORD_START.includes(t[i - 1] ?? "") ? 2 : 0);
    last = i;
    at = i + 1;
  }
  return score;
}

/** `items` matching `query` on their label and detail, the best first (ties keep their order). */
export function rank(items: PaletteItem[], query: string): PaletteItem[] {
  return items
    .map((item) => ({ item, score: fuzzy(query, `${item.label} ${item.detail ?? ""}`) }))
    .filter((e): e is { item: PaletteItem; score: number } => e.score !== null)
    .sort((a, b) => b.score - a.score)
    .map((e) => e.item);
}

/**
 * The palette's commands: every shortcut command but the palette itself, then the commands
 * without a shortcut that apply now, among them the selected worktree's run scripts (6.8).
 */
export function paletteCommands(s: HiveState): PaletteItem[] {
  const project = currentProject(s);
  const extra: PaletteItem[] = [];
  if (project) {
    extra.push({
      label: "Remove merged worktrees…",
      detail: s.projects?.[project]?.name,
      run: () => openModal("remove-merged", project),
    });
  }
  const place = panelWorktree(s);
  if (place) {
    const { worktree } = place;
    for (const script of scriptsOf(s.settings, place.project.id).run) {
      extra.push({
        label: `Run: ${script.name}`,
        detail: worktree.name,
        run: () => void openWith(worktree.path, script.command),
      });
    }
  }
  if (!("why" in reviewTarget(s))) extra.push({ label: "Send review", run: sendReview });
  return [...COMMANDS.filter((c) => c.id !== "palette"), ...extra];
}

/** Agents in tree order (of every space), then the current space's worktrees: Enter selects and shows it. */
export function places(s: HiveState): PaletteItem[] {
  const all = Object.values(s.projects ?? {});
  const worktreeName = (id: string | null) =>
    all.flatMap((p) => p.worktrees).find((w) => w.id === id)?.name;
  const agents = treeAgents(s).map((agent) => {
    const state = s.agentStates[agent.id]?.state;
    const where = worktreeName(agent.worktree) ?? agent.cwd ?? "";
    return {
      label: s.agentTitles[agent.id] ?? "Claude",
      detail: state ? `${STATE_LABEL[state]} · ${where}` : where,
      run: () => goToAgent(agent),
    };
  });
  const worktrees = spaceProjects(s).flatMap((project) =>
    project.worktrees.map((w) => ({
      label: w.name,
      detail: project.name,
      run: () => {
        useHive.setState((s) => ({ collapsed: { ...s.collapsed, [project.id]: false } }));
        select(w.id);
      },
    })),
  );
  return [...agents, ...worktrees];
}

const close = () => openModal(null);

/**
 * Ctrl+Shift+P: commands, agents and worktrees filtered as typed, plus the matching lines of the
 * selected worktree's files (searched by the service once typing pauses). Arrows move, Enter
 * (or a click) runs the row, Esc closes.
 */
export function Palette() {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const q = query.trim();
  const s = useHive();
  const worktree = panelWorktree(s)?.worktree.path ?? null;
  useEffect(() => {
    if (!worktree || !q) return;
    const later = setTimeout(() => void transport.searchFiles(worktree, q), SEARCH_DELAY_MS);
    return () => clearTimeout(later);
  }, [worktree, q]);
  const results =
    worktree && s.searchResults?.worktree === worktree && s.searchResults.query === q
      ? s.searchResults
      : null;
  const files: PaletteItem[] = (results?.matches ?? []).slice(0, FILE_LIMIT).map((m) => ({
    label: `${m.path}:${m.line}`,
    detail: m.text.trim(),
    // As the Files panel opens them: editable text, changed or not.
    run: () => leaveFile({ worktree: results?.worktree ?? "", path: m.path }, true, m.line),
  }));
  const groups: Group[] = [
    { name: "Commands", icon: LightningIcon, items: rank(paletteCommands(s), q) },
    { name: "Agents and worktrees", icon: GitBranchIcon, items: rank(places(s), q) },
    { name: "Files", icon: FileTextIcon, items: files },
  ].filter((g) => g.items.length > 0);
  const items = groups.flatMap((g) => g.items);
  const searching = !!worktree && q !== "" && !results;
  const run = (item: PaletteItem | undefined) => {
    if (!item) return;
    close();
    item.run();
  };
  let n = 0;
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog picker palette"
      aria-label="Command palette"
      ref={(dialog) => {
        if (dialog && !dialog.open) dialog.showModal();
      }}
      onClose={close}
      // A click outside the palette lands on the dialog itself (its backdrop): it closes.
      onClick={(e) => e.target === e.currentTarget && close()}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") setIndex(Math.min(items.length - 1, index + 1));
        else if (e.key === "ArrowUp") setIndex(Math.max(0, index - 1));
        else if (e.key === "Enter") run(items[index]);
        else return;
        e.preventDefault();
      }}
    >
      <label className="picker-search">
        <SearchIcon />
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          placeholder="Type a command, agent, worktree or text in files…"
          aria-label="Search commands, agents, worktrees and files"
          spellCheck={false}
          autoComplete="off"
        />
      </label>
      <div className="picker-list">
        {groups.map((group) => (
          <section key={group.name} aria-label={group.name}>
            <h3 className="palette-group">
              <group.icon {...ICON} />
              {group.name}
            </h3>
            {group.items.map((item) => {
              const i = n++;
              return (
                <button
                  key={`${i}:${item.label}`}
                  type="button"
                  className="picker-row"
                  aria-pressed={i === index}
                  ref={i === index ? (row) => row?.scrollIntoView({ block: "nearest" }) : undefined}
                  onMouseEnter={() => setIndex(i)}
                  onClick={() => run(item)}
                >
                  <span className="picker-name">{item.label}</span>
                  <span className="picker-path">{item.detail}</span>
                  {item.keys && <kbd>{keyText(item.keys)}</kbd>}
                </button>
              );
            })}
          </section>
        ))}
        {searching && <div className="branch-empty">Searching files…</div>}
        {items.length === 0 && !searching && <div className="branch-empty">No matches</div>}
      </div>
      <footer className="picker-hints">
        <span>
          <kbd>↑↓</kbd>navigate
        </span>
        <span>
          <kbd>Enter</kbd>run
        </span>
        <span>
          <kbd>Esc</kbd>close
        </span>
      </footer>
    </dialog>
  );
}
