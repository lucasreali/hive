import { ArrowLeftIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { type CheckState, PULLS_INTERVAL_MS } from "../pulls";
import {
  actOnRun,
  duration,
  hideJobLog,
  hideRun,
  type OpenRun,
  RUN_INTERVAL_MS,
  type RunJob,
  runOf,
  showJobLog,
  showRun,
  words,
} from "../runs";
import { ago } from "../sessions";
import {
  ask,
  type Project,
  runsKey,
  select,
  setPanelView,
  setRightPanel,
  useHive,
  type Worktree,
} from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { ExternalIcon, RefreshIcon } from "./icons";
import { openLink } from "./Markdown";

/** A run's, job's or step's state as one mark, as the pull requests' checks show it. */
const MARK: Record<CheckState, string> = { passing: "✓", failing: "✗", running: "●", skipped: "–" };

const Mark = ({ state, status }: { state: CheckState; status: string }) => (
  <span className="pull-checks" data-state={state} title={words(status)}>
    {MARK[state]}
  </span>
);

/** "3h ago" from GitHub's ISO 8601 time; nothing when it cannot be read. */
const since = (iso: string) => {
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : ago(ms);
};

/**
 * The Actions view (9.32) of the right panel, for the shown worktree's project: its latest
 * workflow runs on one branch (the worktree's first) or all, or one run's jobs. The list is
 * asked for when the view shows and every `PULLS_INTERVAL_MS` while it does; the service never
 * asks GitHub more often. Remount it per worktree (`key`) to start on that worktree's branch.
 */
export function RunsView({ project, worktree }: { project: Project; worktree: Worktree }) {
  const open = useHive((s) => (s.openRun?.project === project.id ? s.openRun : null));
  const [branch, setBranch] = useState(worktree.branch);
  useEffect(() => {
    const ask = () => void transport.listRuns(project.id, branch, false);
    ask();
    const timer = setInterval(ask, PULLS_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [project.id, branch]);
  return open ? (
    <RunDetails open={open} branch={branch} />
  ) : (
    <RunList project={project} worktree={worktree} branch={branch} setBranch={setBranch} />
  );
}

function RunList(props: {
  project: Project;
  worktree: Worktree;
  branch: string | null;
  setBranch: (branch: string | null) => void;
}) {
  const { project, branch } = props;
  const list = useHive((s) => s.runs[runsKey(project.id, branch)]);
  // The shown worktree's branch first, then the project's other worktrees'.
  const branches = new Set([props.worktree.branch, ...project.worktrees.map((w) => w.branch)]);
  const options = [...branches]
    .filter((b) => b !== null)
    .map((b) => ({ value: b, label: b }))
    .concat({ value: "", label: "All branches" });
  return (
    <div className="pulls">
      <div className="pulls-bar">
        <Select
          aria-label="Branch"
          value={branch ?? ""}
          options={options}
          onChange={(b) => props.setBranch(b || null)}
        />
        <span className="pulls-updated">
          {list?.fetched_ms ? `Updated ${ago(list.fetched_ms)}` : ""}
        </span>
        <button
          type="button"
          className="ghost icon"
          title="Refresh"
          onClick={() => void transport.listRuns(project.id, branch, true)}
        >
          <RefreshIcon />
        </button>
      </div>
      {list?.error && (
        <p className="pulls-error" role="alert">
          {list.error}
        </p>
      )}
      {!list && <div className="hint">Loading…</div>}
      {list && !list.error && list.runs.length === 0 && <div className="hint">No runs</div>}
      <ul>
        {list?.runs.map((r) => (
          <li key={r.id} className="session run">
            <button
              type="button"
              className="session-main"
              onClick={() => showRun(project.id, r.id)}
            >
              <span className="session-title">
                <Mark state={r.state} status={r.status} />
                <span className="label">{r.title}</span>
              </span>
              <span className="session-meta">
                {[
                  `${r.workflow} #${r.number}`,
                  branch === null && r.branch,
                  r.event,
                  words(r.status),
                  duration(r.started_at, r.state === "running" ? null : r.updated_at),
                  since(r.created_at),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function RunDetails({ open, branch }: { open: OpenRun; branch: string | null }) {
  const { project, run, detail: d } = open;
  const busy = useHive((s) => s.runBusy?.project === project && s.runBusy.run === run);
  const failed = useHive((s) =>
    s.runError?.project === project && s.runError.run === run ? s.runError.message : null,
  );
  const running = d?.summary.state === "running";
  // A run in progress: its jobs again every 30 s while they show.
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => void transport.openRun(project, run), RUN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [project, run, running]);
  const bar = (
    <div className="pulls-bar">
      <button type="button" className="ghost" onClick={hideRun}>
        <ArrowLeftIcon size={14} aria-hidden="true" />
        Runs
      </button>
      <span className="pulls-updated" />
      <button
        type="button"
        className="ghost icon"
        title="Refresh"
        onClick={() => void transport.openRun(project, run)}
      >
        <RefreshIcon />
      </button>
    </div>
  );
  if (!d) {
    return (
      <div className="pulls">
        {bar}
        {open.error ? (
          <p className="pulls-error" role="alert">
            {open.error}
          </p>
        ) : (
          <div className="hint">Loading the run…</div>
        )}
      </div>
    );
  }
  const r = d.summary;
  const act = (action: Parameters<typeof actOnRun>[2]) => actOnRun(project, run, action, branch);
  const cancel = () =>
    ask({
      title: "Cancel run?",
      text: `Cancel ${r.workflow} #${r.number} on ${r.branch}? Its jobs stop where they are.`,
      action: "Cancel run",
      run: () => act({ kind: "cancel" }),
    });
  return (
    <div className="pulls pull-details">
      {bar}
      <div className="pull-body">
        <h3 className="pull-heading">
          {r.title} <span className="pull-number">#{r.number}</span>
        </h3>
        <div className="session-meta">
          <Mark state={r.state} status={r.status} />{" "}
          {[
            r.workflow,
            r.branch,
            r.event,
            words(r.status),
            duration(r.started_at, running ? null : r.updated_at),
          ]
            .filter(Boolean)
            .join(" · ")}
        </div>
        <div className="pull-actions">
          {r.url && (
            <button type="button" className="secondary" onClick={() => void openLink(r.url)}>
              <ExternalIcon />
              Open on GitHub
            </button>
          )}
          {running ? (
            <button type="button" className="secondary danger" disabled={busy} onClick={cancel}>
              Cancel
            </button>
          ) : (
            <>
              {r.state === "failing" && (
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => act({ kind: "rerun", failed: true })}
                >
                  Re-run failed jobs
                </button>
              )}
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => act({ kind: "rerun", failed: false })}
              >
                Re-run all jobs
              </button>
            </>
          )}
        </div>
        {failed && (
          <p className="pulls-error" role="alert">
            {failed}
          </p>
        )}
        <section aria-label={`Jobs (${d.jobs.length})`}>
          <h4 className="pulls-head">Jobs ({d.jobs.length})</h4>
          {d.jobs.length === 0 ? (
            <div className="hint">None</div>
          ) : (
            <ul>
              {d.jobs.map((job) => (
                <Job key={job.id} project={project} job={job} />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

/** A job with its steps (open when it failed or runs), and a failed one's log tail. */
function Job({ project, job }: { project: string; job: RunJob }) {
  const log = useHive((s) =>
    s.jobLog?.project === project && s.jobLog.job === job.id ? s.jobLog : null,
  );
  const running = job.state === "running";
  return (
    <li className="run-job">
      <details open={job.state === "failing" || running}>
        <summary>
          <Mark state={job.state} status={job.status} />
          <span className="label">{job.name}</span>
          <span className="session-meta">
            {duration(job.started_at, running ? null : job.completed_at)}
          </span>
        </summary>
        <ol className="run-steps">
          {job.steps.map((step) => (
            <li key={step.number} className="run-step">
              <Mark state={step.state} status={step.status} />
              <span className="label">{step.name}</span>
            </li>
          ))}
        </ol>
        {job.state === "failing" &&
          (log ? (
            <>
              <button type="button" className="ghost" onClick={hideJobLog}>
                Hide the log
              </button>
              {log.error && (
                <p className="pulls-error" role="alert">
                  {log.error}
                </p>
              )}
              {!log.error && (
                <pre
                  className="run-log"
                  // The failure is at the end: show it first.
                  ref={(pre) => {
                    if (pre) pre.scrollTop = pre.scrollHeight;
                  }}
                >
                  {log.log ?? "Loading the log…"}
                </pre>
              )}
            </>
          ) : (
            <button type="button" className="ghost" onClick={() => showJobLog(project, job.id)}>
              Show the end of its log
            </button>
          ))}
      </details>
    </li>
  );
}

/**
 * The latest Actions run of a worktree's branch in the sidebar, as one mark; a click shows it
 * in the Actions view.
 */
export function RunBadge({ worktree }: { worktree: string }) {
  const [project, run] = useHive(useShallow((s) => runOf(s, worktree)));
  if (!project || !run) return null;
  return (
    <button
      type="button"
      className="pull-checks run-badge"
      data-state={run.state}
      title={`${run.workflow} #${run.number}: ${words(run.status)}`}
      onClick={() => {
        select(worktree);
        setRightPanel("files");
        setPanelView("actions");
        showRun(project, run.id);
      }}
    >
      {MARK[run.state]}
    </button>
  );
}

/**
 * Asks for the runs of `projects` on every branch once each, for the sidebar's badges: once
 * connected, and again when the projects change or after a reconnection.
 */
export function useRunBadges(projects: string[]) {
  const key = projects.join("\n");
  const connected = useHive((s) => s.connection.status === "connected");
  useEffect(() => {
    if (!connected) return;
    for (const project of key.split("\n").filter(Boolean)) {
      void transport.listRuns(project, null, false);
    }
  }, [key, connected]);
}
