import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ChangedFile } from "../protocol";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { CommitBox } from "./CommitBox";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const login = shop.worktrees[1];

const file = (path: string, staged: ChangedFile["staged"]): ChangedFile => ({
  path,
  status: "modified",
  old_path: null,
  added: 1,
  removed: 0,
  staged,
});

function show(files: ChangedFile[]) {
  apply({ type: "projects", projects: [shop] });
  const changes = { path: login.path, base: "head", branch: null, base_error: null } as const;
  apply({ type: "changes", ...changes, files, added: 1, removed: 0, error: null });
  render(<CommitBox worktree={login} />);
}

const all = () => screen.getByRole("checkbox") as HTMLInputElement;
const message = () => screen.getByLabelText("Commit message") as HTMLTextAreaElement;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;

test("nothing shows before the changes arrive", () => {
  apply({ type: "projects", projects: [shop] });
  render(<CommitBox worktree={login} />);
  expect(screen.queryByRole("region", { name: "Commit" })).toBeNull();
});

test("counts what is staged; its box stages or unstages them all", () => {
  const stage = spyOn(transport, "stageFiles").mockResolvedValue();
  show([file("a", "no"), file("b", "partly"), file("c", null)]);
  expect(screen.getByText("1 of 2 staged")).toBeDefined();
  expect([all().checked, all().indeterminate]).toEqual([false, true]);
  fireEvent.click(all());
  expect(stage).toHaveBeenCalledWith(login.path, ["a", "b"], true);
  // Busy: nothing more until the service answers.
  expect(all().disabled).toBe(true);
  act(() => apply({ type: "git_done", worktree: login.path, action: "stage", message: "" }));
  const done = [file("a", "yes"), file("b", "yes")];
  act(() =>
    apply({
      type: "changes",
      path: login.path,
      base: "head",
      branch: null,
      base_error: null,
      files: done,
      added: 0,
      removed: 0,
      error: null,
    }),
  );
  expect([all().checked, all().indeterminate]).toEqual([true, false]);
  fireEvent.click(all());
  expect(stage).toHaveBeenLastCalledWith(login.path, ["a", "b"], false);
  stage.mockRestore();
});

test("Commit needs a message and staged files, asks first, and empties once done", () => {
  const commit = spyOn(transport, "commit").mockResolvedValue();
  show([file("a", "yes"), file("b", "no")]);
  expect(button("Commit").disabled).toBe(true);
  fireEvent.change(message(), { target: { value: "Fix login" } });
  fireEvent.click(button("Commit"));
  expect(useHive.getState().question?.action).toBe("Commit");
  act(() => useHive.getState().question?.run());
  expect(commit).toHaveBeenCalledWith(login.path, "Fix login");
  expect(button("Committing…").disabled).toBe(true);
  // A hook said no: its output shows, the message stays.
  const said = "lint: 2 problems\nhook says no";
  act(() => apply({ type: "git_failed", worktree: login.path, action: "commit", message: said }));
  expect(screen.getByRole("alert").textContent).toBe(said);
  expect(message().value).toBe("Fix login");
  // A push's failure shows in its dialog, not here.
  act(() => apply({ type: "git_failed", worktree: login.path, action: "push", message: "no" }));
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => useHive.getState().question?.run());
  act(() => apply({ type: "git_done", worktree: login.path, action: "commit", message: "ok" }));
  expect(message().value).toBe("");
  commit.mockRestore();
});

test("Push… opens the push dialog; a detached worktree cannot push", () => {
  const target = spyOn(transport, "getPushTarget").mockResolvedValue();
  show([file("a", "no")]);
  fireEvent.click(button("Push…"));
  expect(useHive.getState()).toMatchObject({ modal: "push", modalWorktree: login.id });
  expect(target).toHaveBeenCalledWith(login.path);
  cleanup();
  render(<CommitBox worktree={{ ...login, branch: null }} />);
  const detached = button("Push…");
  expect(detached.disabled).toBe(true);
  expect(detached.title).toBe("This worktree is on no branch");
  target.mockRestore();
});
