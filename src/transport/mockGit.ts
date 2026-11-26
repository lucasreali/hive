import type { Workflow, WorkflowValue } from "../git";
import type { ChangedFile, Project, ServiceMessage } from "../protocol";

const SHOP = "/home/user/projects/shop";

/** The fake shop repository's workflows that can be run by hand (27.6). */
export const MOCK_WORKFLOWS: Workflow[] = [
  { file: "ci.yml", name: "CI", inputs: [] },
  {
    file: "release.yml",
    name: "Release",
    inputs: [
      {
        name: "level",
        description: "Which part of the version to bump",
        required: true,
        default: "patch",
        kind: "choice",
        options: ["patch", "minor", "major"],
      },
      {
        name: "dry_run",
        description: "Build without publishing",
        required: false,
        default: "true",
        kind: "boolean",
        options: [],
      },
      {
        name: "notes",
        description: "",
        required: false,
        default: null,
        kind: "string",
        options: [],
      },
    ],
  },
];

/** The fake worktree whose remote branch moved on: its push is refused, as git refuses one. */
export const MOCK_REJECTED = "refactor-auth";

/**
 * The Changes panel's git side and "Run workflow…" of the mock transport (27.6), as the service
 * answers: `files` are the fake worktrees' changes, changed in place; `changed` sends a
 * worktree's changes again, as the files watch does after git touched the index; `runs` makes
 * a run for a started workflow and sends it and its branch's list, as the service follows it.
 */
export function createMockGit(
  later: (message: ServiceMessage) => void,
  holder: (path: string) => Project | undefined,
  files: Record<string, ChangedFile[]>,
  changed: (worktree: string) => void,
  runs: {
    dispatch: (project: string, workflow: string, branch: string) => number | null;
    follow: (project: string, run: number | null, branch: string) => void;
  },
) {
  const refused = (path: string) => `${path} is not a worktree of a followed project`;
  const branchOf = (path: string) =>
    holder(path)?.worktrees.find((w) => w.path === path)?.branch ?? null;
  return {
    async stageFiles(worktree: string, paths: string[], staged: boolean) {
      const action = staged ? "stage" : "unstage";
      if (!holder(worktree)) {
        return later({ type: "git_failed", worktree, action, message: refused(worktree) });
      }
      for (const f of files[worktree] ?? []) {
        if (paths.includes(f.path)) f.staged = staged ? "yes" : "no";
      }
      const what = paths.length === 1 ? paths[0] : `${paths.length} files`;
      const message = `${staged ? "Staged" : "Unstaged"} ${what}`;
      later({ type: "git_done", worktree, action, message });
      changed(worktree);
    },
    async commit(worktree: string, message: string) {
      const failed = (why: string) =>
        later({ type: "git_failed", worktree, action: "commit", message: why });
      if (!holder(worktree)) return failed(refused(worktree));
      if (!message.trim()) return failed("A commit message is needed");
      const list = files[worktree] ?? [];
      if (!list.some((f) => f.staged === "yes" || f.staged === "partly")) {
        return failed('no changes added to commit (use "git add" and/or "git commit -a")');
      }
      files[worktree] = list
        .filter((f) => f.staged !== "yes")
        .map((f) => (f.staged === "partly" ? { ...f, staged: "no" } : f));
      const subject = message.trim().split("\n")[0];
      later({
        type: "git_done",
        worktree,
        action: "commit",
        message: `Committed 4f2a9c1: ${subject}`,
      });
      changed(worktree);
    },
    async getPushTarget(worktree: string) {
      const branch = branchOf(worktree);
      const none = { branch: null, target: null, set_upstream: false, ahead: null };
      if (!holder(worktree)) {
        return later({ type: "push_target", worktree, ...none, error: refused(worktree) });
      }
      if (!branch) {
        const error = "This worktree is on no branch (a detached HEAD): nothing to push";
        return later({ type: "push_target", worktree, ...none, error });
      }
      // The main worktree's branch has its upstream; a new worktree's branch has none yet.
      const main = holder(worktree)?.worktrees.find((w) => w.path === worktree)?.main ?? false;
      const target = `origin/${branch}`;
      const ahead = main ? 1 : null;
      later({
        type: "push_target",
        worktree,
        branch,
        target,
        set_upstream: !main,
        ahead,
        error: null,
      });
    },
    async push(worktree: string) {
      const branch = branchOf(worktree);
      const failed = (message: string) =>
        later({ type: "git_failed", worktree, action: "push", message });
      if (!holder(worktree)) return failed(refused(worktree));
      if (!branch)
        return failed("This worktree is on no branch (a detached HEAD): nothing to push");
      if (worktree.endsWith(`/${MOCK_REJECTED}`)) {
        return failed(
          [
            "To github.com:user/api.git",
            ` ! [rejected]        ${branch} -> ${branch} (fetch first)`,
            "error: failed to push some refs to 'github.com:user/api.git'",
            "hint: Updates were rejected because the remote contains work that you do not",
            "hint: have locally.",
          ].join("\n"),
        );
      }
      later({
        type: "git_done",
        worktree,
        action: "push",
        message: `Pushed ${branch} to origin/${branch}`,
      });
    },
    async listWorkflows(project: string, worktree: string) {
      const workflows = project === SHOP && holder(worktree)?.id === project ? MOCK_WORKFLOWS : [];
      later({
        type: "workflows",
        project,
        worktree,
        workflows: structuredClone(workflows),
        error: null,
      });
    },
    async runWorkflow(project: string, workflow: string, branch: string, _inputs: WorkflowValue[]) {
      const found = project === SHOP && MOCK_WORKFLOWS.find((w) => w.file === workflow);
      if (!found) {
        const message = `gh workflow run failed: HTTP 404: could not find workflow ${workflow}`;
        return later({ type: "workflow_failed", project, message });
      }
      const run = runs.dispatch(project, found.name, branch);
      const message = `Started ${workflow} on ${branch}`;
      later({ type: "workflow_started", project, branch, run, message });
      runs.follow(project, run, branch);
    },
  };
}
