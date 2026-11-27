import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { notice } from "../../test/notice";
import { App } from "../App";
import { openPush, type PushTarget } from "../git";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const login = shop.worktrees[1];

function open() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => openPush(shop.id, login));
  return screen.getByRole("dialog", { name: "Push" });
}

const target = (extra: Partial<PushTarget>): PushTarget => ({
  worktree: login.path,
  branch: login.branch,
  target: `origin/${login.branch}`,
  at: { remote: "origin", merge: `refs/heads/${login.branch}` },
  set_upstream: false,
  ahead: 2,
  error: null,
  ...extra,
});

test("says where the branch goes, then pushes once asked", () => {
  const ask = spyOn(transport, "getPushTarget").mockResolvedValue();
  const push = spyOn(transport, "push").mockResolvedValue();
  const dialog = open();
  const submit = within(dialog).getByRole("button", { name: "Push" }) as HTMLButtonElement;
  expect(dialog.textContent).toContain(`Looking for where ${login.branch} goes…`);
  expect(submit.disabled).toBe(true);
  act(() => apply({ type: "push_target", ...target({}) }));
  expect(dialog.textContent).toContain(
    `Push ${login.branch} to origin/${login.branch}? 2 commits to send.`,
  );
  act(() => apply({ type: "push_target", ...target({ ahead: 1 }) }));
  expect(dialog.textContent).toContain("? 1 commit to send.");
  act(() => apply({ type: "push_target", ...target({ ahead: 0 }) }));
  expect(dialog.textContent).toContain("? It has no commit to send.");
  act(() => apply({ type: "push_target", ...target({ set_upstream: true, ahead: null }) }));
  expect(dialog.textContent).toContain(", and make it its upstream?");
  fireEvent.click(submit);
  expect(push).toHaveBeenCalledWith(login.path, login.branch, {
    remote: "origin",
    merge: `refs/heads/${login.branch}`,
  });
  expect(submit.textContent).toBe("Pushing…");
  expect(submit.disabled).toBe(true);
  // Rejected: git's message as is, and Push again.
  const rejected = " ! [rejected]        main -> main (fetch first)";
  act(() => apply({ type: "git_failed", worktree: login.path, action: "push", message: rejected }));
  expect(within(dialog).getByRole("alert").textContent).toBe(rejected);
  expect(submit.textContent).toBe("Push");
  fireEvent.click(submit);
  act(() => apply({ type: "git_done", worktree: login.path, action: "push", message: "Pushed" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(notice()).toBe("Pushed");
  ask.mockRestore();
  push.mockRestore();
});

test("a branch that cannot be pushed says why; Cancel and the X close", () => {
  const ask = spyOn(transport, "getPushTarget").mockResolvedValue();
  const dialog = open();
  const why = "This worktree is on no branch (a detached HEAD): nothing to push";
  act(() =>
    apply({ type: "push_target", ...target({ branch: null, target: null, at: null, error: why }) }),
  );
  expect(within(dialog).getByRole("alert").textContent).toBe(why);
  expect((within(dialog).getByRole("button", { name: "Push" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  act(() => openPush(shop.id, login));
  const again = screen.getByRole("dialog", { name: "Push" });
  fireEvent.click(within(again).getByTitle("Close (Esc)"));
  expect(useHive.getState().modal).toBeNull();
  // A worktree that is gone shows nothing.
  act(() => openPush(shop.id, { id: "/gone", path: "/gone" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  ask.mockRestore();
});
