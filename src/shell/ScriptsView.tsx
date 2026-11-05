import { PlayIcon, StopIcon, TerminalWindowIcon } from "@phosphor-icons/react";
import { useEffect } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Project, Script, ScriptSource, Worktree } from "../protocol";
import { runningScripts, runScript, stopScript } from "../scripts";
import { activateTab, openModal, useHive } from "../store";
import { showOpenFailure } from "../terminals";
import { transport } from "../transport";

const ICON = { size: 14, "aria-hidden": true } as const;

/** The groups, in the order the service lists them. */
const SOURCES: { source: ScriptSource; label: string }[] = [
  { source: "settings", label: "Settings" },
  { source: "package_json", label: "package.json" },
  { source: "taskfile", label: "Taskfile" },
  { source: "makefile", label: "Makefile" },
];

/** Ports each worktree gets, from `HIVE_PORT` on (`hive::scripts::BLOCK`). */
const BLOCK = 10;

/**
 * The right panel's Scripts tab (23.4): the shown worktree's port block, the scripts started
 * here that still run (show their terminal, stop with Ctrl+C), then every script by source; a
 * click types one into a new terminal in the worktree. The list is asked for when shown and
 * follows the service's changes.
 */
export function ScriptsView({ project, worktree }: { project: Project; worktree: Worktree }) {
  const path = worktree.path;
  const listed = useHive((s) => s.scripts[path]);
  const shown = useHive(
    useShallow((s) => ({ scriptRuns: s.scriptRuns, tabs: s.tabs, terminals: s.terminals })),
  );
  const running = runningScripts(shown, path);
  useEffect(() => {
    void transport.listScripts(path);
  }, [path]);
  const run = (script: Script) => showOpenFailure(runScript(path, script));
  const show = (id: number) => {
    const tab = useHive.getState().tabs.find((t) => t.id === id);
    if (tab) activateTab(tab);
  };
  const settings = () => openModal("settings", project.id);
  const groups = SOURCES.map((g) => ({
    ...g,
    scripts: (listed?.scripts ?? []).filter((s) => s.source === g.source),
  })).filter((g) => g.scripts.length > 0);
  const port = listed?.port ?? null;
  return (
    <div className="scripts">
      {port !== null && (
        <div className="scripts-ports" title="Every terminal Hive opens in this worktree has these">
          <span>
            Ports {port}–{port + BLOCK - 1}
          </span>
          <code className="scripts-code">$PORT {port}</code>
          <code className="scripts-code">$HIVE_PORT {port}</code>
        </div>
      )}
      {running.length > 0 && (
        <section className="scripts-group" aria-label="Running">
          <h3>Running</h3>
          <ul>
            {running.map((r) => (
              <li key={r.id} className="script running">
                <span className="script-dot" aria-hidden="true" />
                <span className="script-name">{r.name}</span>
                <span className="script-command">{r.command}</span>
                <button
                  type="button"
                  className="ghost icon"
                  title={`Show the terminal of ${r.name}`}
                  onClick={() => show(r.id)}
                >
                  <TerminalWindowIcon {...ICON} />
                </button>
                <button
                  type="button"
                  className="ghost icon"
                  title={`Stop ${r.name} (sends Ctrl+C)`}
                  onClick={() => stopScript(r.id)}
                >
                  <StopIcon {...ICON} />
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {!listed && <div className="hint">Loading…</div>}
      {listed && groups.length === 0 && (
        <div className="hint scripts-empty">
          <span>
            No scripts. Add this project's own in Settings, or a{" "}
            <code className="scripts-code">package.json</code>,{" "}
            <code className="scripts-code">Taskfile.yml</code> or{" "}
            <code className="scripts-code">Makefile</code> to the repository.
          </span>
          <button type="button" className="secondary" onClick={settings}>
            Open Settings
          </button>
        </div>
      )}
      {groups.map((g) => (
        <section key={g.source} className="scripts-group" aria-label={g.label}>
          <h3>{g.label}</h3>
          <ul>
            {g.scripts.map((script) => (
              <li key={script.name} className="script">
                <button
                  type="button"
                  className="script-main"
                  title={`Run in a new terminal: ${script.command}`}
                  onClick={() => run(script)}
                >
                  <PlayIcon {...ICON} className="script-play" />
                  <span className="script-name">{script.name}</span>
                  <span className="script-command">{script.command}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {listed?.problems.map((p) => (
        <p key={p} className="scripts-problem">
          {p}
        </p>
      ))}
      {listed && groups.length > 0 && (
        <div className="hint">
          Nothing runs until you click it.{" "}
          <button type="button" className="link" onClick={settings}>
            Edit this project's scripts in Settings
          </button>
        </div>
      )}
    </div>
  );
}
