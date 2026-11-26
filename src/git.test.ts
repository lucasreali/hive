import { afterEach, expect, spyOn, test } from "bun:test";
import { notice } from "../test/notice";
import { askCommit, openPush, openRunWorkflow, push, runWorkflow, stageFiles } from "./git";
import { apply } from "./reduce";
import { initialState, useHive } from "./store";
import { transport } from "./transport";

afterEach(() => useHive.setState(initialState, true));

const w = { id: "/r/w", path: "/r/w" };

test("staging sends the paths and marks the worktree busy; none sends nothing", () => {
  const stage = spyOn(transport, "stageFiles").mockResolvedValue();
  stageFiles(w.path, [], true);
  expect(stage).not.toHaveBeenCalled();
  stageFiles(w.path, ["a", "b"], true);
  expect(stage).toHaveBeenCalledWith(w.path, ["a", "b"], true);
  expect(useHive.getState().git[w.path]).toEqual({ busy: "stage", error: null });
  // Done: no toast for staging, the list itself shows it.
  apply({ type: "git_done", worktree: w.path, action: "stage", message: "Staged 2 files" });
  expect(useHive.getState().git[w.path]).toEqual({ busy: null, error: null });
  expect(notice()).toBeNull();
  stageFiles(w.path, ["a"], false);
  expect(useHive.getState().git[w.path]?.busy).toBe("unstage");
  apply({ type: "git_failed", worktree: w.path, action: "unstage", message: "fatal: x" });
  expect(useHive.getState().git[w.path]).toEqual({
    busy: null,
    error: { action: "unstage", message: "fatal: x" },
  });
  stage.mockRestore();
});

test("a commit is asked first, naming the files, the branch and the subject", () => {
  const commit = spyOn(transport, "commit").mockResolvedValue();
  askCommit(w.path, "topic", 2, "Fix it\n\nbecause");
  const { question, modal } = useHive.getState();
  expect(modal).toBe("confirm");
  expect(question?.title).toBe("Commit?");
  expect(question?.text).toBe(
    "Commit 2 staged files on topic as “Fix it”? The repository's hooks run as git runs them.",
  );
  expect(commit).not.toHaveBeenCalled();
  question?.run();
  expect(commit).toHaveBeenCalledWith(w.path, "Fix it\n\nbecause");
  expect(useHive.getState().git[w.path]?.busy).toBe("commit");
  apply({ type: "git_done", worktree: w.path, action: "commit", message: "Committed abc: Fix it" });
  expect(notice()).toBe("Committed abc: Fix it");
  askCommit(w.path, null, 1, "x");
  expect(useHive.getState().question?.text).toStartWith("Commit 1 staged file as “x”?");
  commit.mockRestore();
});

test("the push dialog asks where the branch goes, pushes, and closes once pushed", () => {
  const target = spyOn(transport, "getPushTarget").mockResolvedValue();
  const send = spyOn(transport, "push").mockResolvedValue();
  useHive.setState({
    pushTargets: { [w.path]: {} as never },
    git: { [w.path]: { busy: null, error: { action: "push", message: "old" } } },
  });
  openPush("/r", w);
  expect(useHive.getState()).toMatchObject({
    modal: "push",
    modalProject: "/r",
    modalWorktree: w.id,
    pushTargets: {},
    git: { [w.path]: { busy: null, error: null } },
  });
  expect(target).toHaveBeenCalledWith(w.path);
  const answer = {
    worktree: w.path,
    branch: "topic",
    target: "origin/topic",
    set_upstream: true,
    ahead: null,
    error: null,
  };
  apply({ type: "push_target", ...answer });
  expect(useHive.getState().pushTargets[w.path]).toEqual(answer);
  push(w.path);
  expect(send).toHaveBeenCalledWith(w.path);
  expect(useHive.getState().git[w.path]?.busy).toBe("push");
  apply({ type: "git_done", worktree: w.path, action: "push", message: "Pushed topic" });
  expect(useHive.getState().modal).toBeNull();
  expect(notice()).toBe("Pushed topic");
  // Pushed with another dialog open: that one stays.
  useHive.setState({ modal: "settings" });
  apply({ type: "git_done", worktree: w.path, action: "push", message: "Pushed topic" });
  expect(useHive.getState().modal).toBe("settings");
  target.mockRestore();
  send.mockRestore();
});

test("Run workflow lists, starts, and follows the new run", () => {
  const list = spyOn(transport, "listWorkflows").mockResolvedValue();
  const run = spyOn(transport, "runWorkflow").mockResolvedValue();
  useHive.setState({ workflowError: "old", workflowBusy: true });
  openRunWorkflow("/r", w);
  expect(useHive.getState()).toMatchObject({
    modal: "run-workflow",
    modalProject: "/r",
    modalWorktree: w.id,
    workflows: null,
    workflowBusy: false,
    workflowError: null,
  });
  expect(list).toHaveBeenCalledWith("/r", w.path);
  const listed = { project: "/r", worktree: w.path, workflows: [], error: null };
  apply({ type: "workflows", ...listed });
  expect(useHive.getState().workflows).toEqual(listed);
  runWorkflow("/r", "ci.yml", "main", [{ name: "a", value: "b" }]);
  expect(run).toHaveBeenCalledWith("/r", "ci.yml", "main", [{ name: "a", value: "b" }]);
  expect(useHive.getState().workflowBusy).toBe(true);
  apply({ type: "workflow_failed", project: "/r", message: "HTTP 422" });
  expect(useHive.getState()).toMatchObject({ workflowBusy: false, workflowError: "HTTP 422" });
  expect(useHive.getState().modal).toBe("run-workflow");
  apply({ type: "workflow_started", project: "/r", branch: "main", run: 9, message: "Started" });
  expect(useHive.getState()).toMatchObject({
    modal: null,
    workflowBusy: false,
    workflowError: null,
    openRun: { project: "/r", run: 9, detail: null, error: null },
  });
  expect(notice()).toBe("Started");
  // Not found yet: started all the same, the list stays shown.
  useHive.setState({ openRun: null, modal: "settings" });
  apply({ type: "workflow_started", project: "/r", branch: "main", run: null, message: "S" });
  expect(useHive.getState()).toMatchObject({ openRun: null, modal: "settings" });
  list.mockRestore();
  run.mockRestore();
});
