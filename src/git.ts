import { ask, openModal, useHive } from "./store";
import { transport } from "./transport";

// Mirror `hive_protocol`'s commit, push and workflow types (27.6, D27.3). Every action here is a
// click of the user's, asked first; the service runs git and `gh`.

/** Which git action of the Changes panel a `git_done` or `git_failed` answers. */
export type GitAction = "stage" | "unstage" | "commit" | "push";
/** What of a changed file the next commit takes. */
export type Staged = "no" | "partly" | "yes";
/** The `push_target` message: where a push of the worktree's branch goes, or why it cannot. */
export type PushTarget = {
  worktree: string;
  branch: string | null;
  /** e.g. `origin/main`. */
  target: string | null;
  /** The branch has no upstream yet: the push sets it. */
  set_upstream: boolean;
  /** Its commits the remote-tracking branch has not; null when unknown. */
  ahead: number | null;
  error: string | null;
};
export type WorkflowInput = {
  name: string;
  description: string;
  required: boolean;
  default: string | null;
  kind: "string" | "boolean" | "choice" | "number" | "environment";
  options: string[];
};
export type Workflow = { file: string; name: string; inputs: WorkflowInput[] };
export type WorkflowValue = { name: string; value: string };
/** The `workflows` message: the ones a worktree's `HEAD` lets run by hand. */
export type Workflows = {
  project: string;
  worktree: string;
  workflows: Workflow[];
  error: string | null;
};
/** A worktree's git action sent and not answered yet, and why the last one failed. */
export type GitState = {
  busy: GitAction | null;
  error: { action: GitAction; message: string } | null;
};

const setGit = (worktree: string, state: GitState) =>
  useHive.setState((s) => ({ git: { ...s.git, [worktree]: state } }));

/** Stages (or unstages) `paths` of `worktree`; the files watch brings the new changes. */
export function stageFiles(worktree: string, paths: string[], staged: boolean): void {
  if (paths.length === 0) return;
  setGit(worktree, { busy: staged ? "stage" : "unstage", error: null });
  void transport.stageFiles(worktree, paths, staged);
}

/** Commits what is staged in `worktree` with `message`, once the user says so. */
export function askCommit(
  worktree: string,
  branch: string | null,
  staged: number,
  message: string,
) {
  const files = staged === 1 ? "1 staged file" : `${staged} staged files`;
  const subject = message.trim().split("\n")[0] ?? "";
  ask({
    title: "Commit?",
    text: `Commit ${files}${branch ? ` on ${branch}` : ""} as “${subject}”? The repository's hooks run as git runs them.`,
    action: "Commit",
    run: () => {
      setGit(worktree, { busy: "commit", error: null });
      void transport.commit(worktree, message);
    },
  });
}

/** A worktree as the dialogs name it (`id`) and the service does (`path`). */
type Place = { id: string; path: string };

/** Opens the push dialog for worktree `w` and asks where its branch would go. */
export function openPush(project: string, w: Place): void {
  useHive.setState((s) => {
    const { [w.path]: _, ...targets } = s.pushTargets;
    const git = { ...s.git, [w.path]: { busy: null, error: null } };
    return { pushTargets: targets, git };
  });
  openModal("push", project, w.id);
  void transport.getPushTarget(w.path);
}

/** Pushes `worktree`'s branch to `target` as the push dialog showed it (the dialog asks). */
export function push(target: PushTarget & { branch: string; target: string }): void {
  setGit(target.worktree, { busy: "push", error: null });
  void transport.push(target.worktree, target.branch, target.target);
}

/** Opens "Run workflow…" for `project`, its workflows as worktree `w`'s `HEAD` has them. */
export function openRunWorkflow(project: string, w: Place): void {
  useHive.setState({ workflows: null, workflowBusy: false, workflowError: null });
  openModal("run-workflow", project, w.id);
  void transport.listWorkflows(project, w.path);
}

/** Runs `workflow` on `branch` with `inputs`; the dialog closes on `workflow_started`. */
export function runWorkflow(
  project: string,
  workflow: string,
  branch: string,
  inputs: WorkflowValue[],
): void {
  useHive.setState({ workflowBusy: true, workflowError: null });
  void transport.runWorkflow(project, workflow, branch, inputs);
}
