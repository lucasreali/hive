import { ArrowLeftIcon } from "@phosphor-icons/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Project, Worktree } from "../protocol";
import {
  actOnPull,
  type CheckState,
  hidePull,
  type MergeMethod,
  newPull,
  type OpenPull,
  PULLS_INTERVAL_MS,
  type PullRepo,
  type PullState,
  type PullSummary,
  pullOf,
  type ReviewDecision,
  type ReviewState,
  setPullBusy,
  showPull,
} from "../pulls";
import { showRun } from "../runs";
import { ago } from "../sessions";
import { ask, scriptsOf, select, setPanelView, setRightPanel, useHive } from "../store";
import { openWith, showOpenFailure } from "../terminals";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { ExternalIcon, RefreshIcon } from "./icons";
import { Markdown, openLink } from "./Markdown";

const STATE: Record<PullState, string> = {
  open: "Open",
  draft: "Draft",
  merged: "Merged",
  closed: "Closed",
};
const REVIEW: Record<ReviewDecision, string> = {
  approved: "Approved",
  changes_requested: "Changes requested",
  review_required: "Review required",
};
const VERDICT: Record<ReviewState, string> = {
  approved: "approved",
  changes_requested: "requested changes",
  commented: "reviewed",
  dismissed: "review dismissed",
};
/** Checks as one mark (9.31: ✓/✗/running), explained by its tooltip. */
const CHECK: Record<CheckState, { mark: string; label: string }> = {
  passing: { mark: "✓", label: "Checks passing" },
  failing: { mark: "✗", label: "Checks failing" },
  running: { mark: "●", label: "Checks running" },
  skipped: { mark: "–", label: "Checks skipped" },
};
const METHOD: Record<MergeMethod, string> = {
  merge: "Create a merge commit",
  squash: "Squash and merge",
  rebase: "Rebase and merge",
};

/** "3h ago" from GitHub's ISO 8601 time; nothing when it cannot be read. */
const since = (iso: string) => {
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : ago(ms);
};

const StateLabel = ({ state }: { state: PullState }) => (
  <span className="pull-state" data-state={state}>
    {STATE[state]}
  </span>
);

const Checks = ({ state }: { state: CheckState | null }) =>
  state && (
    <span className="pull-checks" data-state={state} title={CHECK[state].label}>
      {CHECK[state].mark}
    </span>
  );

/**
 * The Pull requests view (9.31) of the right panel, for the shown worktree's project: the
 * space account's pull requests and those asking for its review, or one's details. The list
 * is asked for when the view shows and every `PULLS_INTERVAL_MS` while it does; the service
 * never asks GitHub more often, and says when GitHub's rate limit holds.
 */
export function PullsView({ project, worktree }: { project: Project; worktree: Worktree }) {
  const open = useHive((s) => (s.openPull?.project === project.id ? s.openPull : null));
  const repo = useHive((s) => s.pulls[project.id]?.repo ?? null);
  useEffect(() => {
    const ask = () => void transport.listPulls(project.id, false);
    ask();
    const timer = setInterval(ask, PULLS_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [project.id]);
  useCheckedOut();
  return open ? (
    <PullDetails open={open} repo={repo} />
  ) : (
    <PullList project={project} worktree={worktree} />
  );
}

/**
 * A pull request checked out from here gets 4.x's new worktree treatment: the project's setup
 * script (6.8) runs in a terminal of its own, and the worktree is selected.
 */
function useCheckedOut() {
  const created = useHive((s) => s.worktreeDialog.created);
  const seen = useRef(created);
  useEffect(() => {
    if (!created || seen.current === created) return;
    seen.current = created;
    const s = useHive.getState();
    if (s.pullBusy?.action !== "checkout" || s.pullBusy.project !== created.project) return;
    const setup = scriptsOf(s.settings, created.project).setup;
    if (setup) showOpenFailure(openWith(created.path, setup));
    select(created.path);
    setPullBusy(null);
  }, [created]);
}

function PullList({ project, worktree }: { project: Project; worktree: Worktree }) {
  const pulls = useHive((s) => s.pulls[project.id]);
  const own = useHive((s) => pullOf(s, worktree.id).length > 0);
  const section = (title: string, list: PullSummary[], by: boolean) => (
    <section aria-label={title}>
      <h3 className="pulls-head">{title}</h3>
      {list.length === 0 ? (
        <div className="hint">None</div>
      ) : (
        <ul>
          {list.map((p) => (
            <PullRow key={p.number} project={project.id} pull={p} by={by} here={worktree.id} />
          ))}
        </ul>
      )}
    </section>
  );
  return (
    <div className="pulls">
      <div className="pulls-bar">
        {pulls?.repo ? (
          <button
            type="button"
            className="link"
            onClick={() => void openLink(pulls.repo?.url ?? "")}
          >
            {pulls.repo.name}
          </button>
        ) : (
          <span>{project.name}</span>
        )}
        <span className="pulls-updated">
          {pulls?.fetched_ms ? `Updated ${ago(pulls.fetched_ms)}` : ""}
        </span>
        <button
          type="button"
          className="ghost icon"
          title="Refresh"
          onClick={() => void transport.listPulls(project.id, true)}
        >
          <RefreshIcon />
        </button>
      </div>
      {pulls?.error && (
        <p className="pulls-error" role="alert">
          {pulls.error}
        </p>
      )}
      {pulls?.repo && worktree.branch && !own && (
        <button
          type="button"
          className="secondary pulls-create"
          title={`Create pull request from ${worktree.branch}`}
          onClick={() => newPull(project.id, worktree.id)}
        >
          Create pull request from <span className="pulls-create-branch">{worktree.branch}</span>
        </button>
      )}
      {!pulls && <div className="hint">Loading…</div>}
      {pulls?.repo && section("Yours", pulls.mine, false)}
      {pulls?.repo && section("Review requested", pulls.review, true)}
    </div>
  );
}

function PullRow(props: { project: string; pull: PullSummary; by: boolean; here: string }) {
  const { pull: p } = props;
  const meta = [
    `#${p.number}`,
    props.by && p.author,
    p.review && REVIEW[p.review],
    since(p.updated_at),
  ];
  return (
    <li className="session pull" data-here={p.worktree === props.here}>
      <button
        type="button"
        className="session-main"
        title={`${p.branch} → ${p.base}`}
        onClick={() => showPull(props.project, p.number)}
      >
        <span className="session-title">
          <StateLabel state={p.state} />
          <span className="label">{p.title}</span>
          <Checks state={p.checks} />
        </span>
        <span className="session-meta">{meta.filter(Boolean).join(" · ")}</span>
      </button>
    </li>
  );
}

function PullDetails({ open, repo }: { open: OpenPull; repo: PullRepo | null }) {
  const { project, number, detail: d } = open;
  const busy = useHive((s) => s.pullBusy?.project === project && s.pullBusy.number === number);
  const failed = useHive((s) =>
    s.pullError?.project === project && s.pullError.number === number ? s.pullError.message : null,
  );
  const methods = repo?.merge_methods ?? [];
  const [method, setMethod] = useState<MergeMethod | null>(null);
  const chosen = method ?? repo?.default_merge ?? methods[0] ?? "merge";
  const bar = (
    <div className="pulls-bar">
      <button type="button" className="ghost" onClick={hidePull}>
        <ArrowLeftIcon size={14} aria-hidden="true" />
        Pull requests
      </button>
      <span className="pulls-updated" />
      <button
        type="button"
        className="ghost icon"
        title="Refresh"
        onClick={() => void transport.openPull(project, number)}
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
          <div className="hint">Loading #{number}…</div>
        )}
      </div>
    );
  }
  const p = d.summary;
  const live = p.state === "open" || p.state === "draft";
  const act = (action: Parameters<typeof actOnPull>[2]) => actOnPull(project, number, action);
  const merge = () =>
    ask({
      title: "Merge pull request?",
      text: `Merge #${number} "${p.title}" into ${p.base} (${METHOD[chosen].toLowerCase()})?`,
      action: "Merge",
      run: () => act({ kind: "merge", method: chosen, head: d.head }),
    });
  const close = () =>
    ask({
      title: "Close pull request?",
      text: `Close #${number} "${p.title}" without merging it?`,
      action: "Close",
      run: () => act({ kind: "close" }),
    });
  const worktree = p.worktree;
  return (
    <div className="pulls pull-details">
      {bar}
      <div className="pull-body">
        <h3 className="pull-heading">
          {p.title} <span className="pull-number">#{number}</span>
        </h3>
        <div className="session-meta">
          <StateLabel state={p.state} /> {p.author} · {p.branch} → {p.base} ·{" "}
          <span className="count-added">+{d.additions}</span>{" "}
          <span className="count-removed">−{d.deletions}</span>
          {p.review && ` · ${REVIEW[p.review]}`}
          {d.conflicts && " · Conflicts with the base"}
        </div>
        <div className="pull-actions">
          {p.url && (
            <button type="button" className="secondary" onClick={() => void openLink(p.url)}>
              <ExternalIcon />
              Open on GitHub
            </button>
          )}
          {worktree ? (
            <button type="button" className="secondary" onClick={() => select(worktree)}>
              Show its worktree
            </button>
          ) : (
            live && (
              <button
                type="button"
                className="secondary"
                disabled={busy}
                title="In a new worktree on its branch"
                onClick={() => act({ kind: "checkout" })}
              >
                Check out
              </button>
            )
          )}
          {p.state === "draft" && (
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => act({ kind: "ready" })}
            >
              Ready for review
            </button>
          )}
          {p.state === "open" && methods.length > 0 && (
            <span className="pull-merge">
              <Select
                aria-label="Merge method"
                value={chosen}
                options={methods.map((m) => ({ value: m, label: METHOD[m] }))}
                onChange={(m) => setMethod(m as MergeMethod)}
                disabled={busy}
              />
              <button type="button" className="primary" disabled={busy} onClick={merge}>
                Merge
              </button>
            </span>
          )}
          {live && (
            <button type="button" className="secondary danger" disabled={busy} onClick={close}>
              Close
            </button>
          )}
        </div>
        {failed && (
          <p className="pulls-error" role="alert">
            {failed}
          </p>
        )}
        <Markdown text={d.body || "*No description.*"} />
        <Part title={`Checks (${d.checks.length})`}>
          {d.checks.map((c) => (
            <li key={`${c.workflow}/${c.name}`} className="pull-check">
              <Checks state={c.state} />
              {c.run !== null || c.url ? (
                <button
                  type="button"
                  className="link"
                  onClick={() => {
                    // An Actions job: its run in the Actions view (9.32).
                    if (c.run === null) return void openLink(c.url ?? "");
                    setPanelView("actions");
                    showRun(project, c.run);
                  }}
                >
                  {c.name}
                </button>
              ) : (
                <span>{c.name}</span>
              )}
              {c.workflow && <span className="session-meta">{c.workflow}</span>}
            </li>
          ))}
        </Part>
        <Part title={`Files (${d.files.length})`}>
          {d.files.map((f) => (
            <li key={f.path} className="pull-file">
              <span className="label">{f.path}</span>
              <span className="count-added">+{f.additions}</span>
              <span className="count-removed">−{f.deletions}</span>
            </li>
          ))}
        </Part>
        <Part title={`Reviews and comments (${d.notes.length})`}>
          {d.notes.map((n) => (
            // Notes have no id of their own: who wrote it, when, and its verdict.
            <li key={`${n.author}\n${n.at}\n${n.review}`} className="pull-note">
              <div className="session-meta">
                <b>{n.author}</b>
                {n.review && ` ${VERDICT[n.review]}`} · {since(n.at)}
              </div>
              {n.body && <Markdown text={n.body} />}
            </li>
          ))}
        </Part>
      </div>
    </div>
  );
}

function Part({ title, children }: { title: string; children: ReactNode[] }) {
  return (
    <section aria-label={title}>
      <h4 className="pulls-head">{title}</h4>
      {children.length === 0 ? <div className="hint">None</div> : <ul>{children}</ul>}
    </section>
  );
}

/**
 * A worktree's pull request in the sidebar (number and state), from the lists last sent;
 * a click shows its details in the Pull requests view.
 */
export function PullBadge({ worktree }: { worktree: string }) {
  const [project, pull] = useHive(useShallow((s) => pullOf(s, worktree)));
  if (!project || !pull) return null;
  return (
    <button
      type="button"
      className="pull-badge"
      data-state={pull.state}
      title={`Pull request #${pull.number}: ${STATE[pull.state]}`}
      onClick={() => {
        select(worktree);
        setRightPanel("files");
        setPanelView("pulls");
        showPull(project, pull.number);
      }}
    >
      #{pull.number}
    </button>
  );
}

/**
 * Asks for the pull requests of `projects` once each, for the sidebar's badges: once
 * connected, and again when the projects change or after a reconnection.
 */
export function usePullBadges(projects: string[]) {
  const key = projects.join("\n");
  const connected = useHive((s) => s.connection.status === "connected");
  useEffect(() => {
    if (!connected) return;
    for (const project of key.split("\n").filter(Boolean)) void transport.listPulls(project, false);
  }, [key, connected]);
}
