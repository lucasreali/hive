import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { notice, noticeKind } from "../../test/notice";
import { App } from "../App";
import type { Project, ReviewPlan, Worktree } from "../protocol";
import { apply } from "../reduce";
import { initialState, openModal, select, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS, MOCK_REVIEW_COMMITS } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const [main, login, checkout] = shop.worktrees;
const reviewPath = `${shop.path}/.claude/worktrees/review-fix-login`;
const review: Worktree = {
  ...login,
  id: reviewPath,
  path: reviewPath,
  name: "review-fix-login",
  branch: "review-fix-login",
  review_of: login.id,
  status: null,
};
const detached = { ...checkout, branch: null };
const project: Project = { ...shop, worktrees: [main, login, detached, review] };

function show(p = project) {
  render(<App />);
  act(() => apply({ type: "projects", projects: [p] }));
}

const tree = () => screen.getByRole("navigation", { name: "Projects" });
const row = (name: string) =>
  within(tree())
    .getAllByRole("button", { name: new RegExp(`^${name}`) })
    .find((b) => b.classList.contains("row-main")) as HTMLElement;
const item = (name: string) => screen.getByRole("menuitem", { name }) as HTMLButtonElement;
const items = () => screen.getAllByRole("menuitem").map((i) => i.textContent);
const rightClick = (name: string) => fireEvent.contextMenu(row(name), { clientX: 4, clientY: 4 });

test("a review shows under the worktree it reviews, marked, and collapses with it", () => {
  show();
  const rows = [...tree().querySelectorAll(".tree-row")].map((r) => r.textContent);
  // After its reviewed worktree, not in the project's own order.
  expect(rows).toEqual([
    "shop",
    "main",
    "fix-login↑3↓1+4−1",
    "review-fix-loginreview",
    "feat-checkout↑1+48−2",
  ]);
  const badge = tree().querySelector(".review-badge") as HTMLElement;
  expect(badge.title).toBe("A review of fix-login");
  expect(row("review-fix-login").closest("li")?.className).toBe("in-review");
  expect(row("fix-login").closest("li")?.className).toBe("");

  // An agent in the review rolls up into its reviewed worktree once collapsed.
  act(() => {
    apply({
      type: "agent_detected",
      channel: 1,
      id: "a",
      project: shop.id,
      worktree: reviewPath,
      cwd: reviewPath,
    });
    apply({ type: "agent_state", id: "a", ...agentStatus("waiting_you"), subagents: [] });
  });
  fireEvent.click(screen.getByRole("button", { name: "Collapse fix-login" }));
  expect(tree().textContent).not.toContain("review-fix-login");
  const rollup = row("fix-login").closest(".tree-row")?.querySelector(".state-icon");
  expect(rollup?.getAttribute("data-state")).toBe("waiting_you");
  fireEvent.click(screen.getByRole("button", { name: "Expand fix-login" }));
  expect(row("review-fix-login")).toBeDefined();
});

test("a review whose reviewed worktree is not shown is a worktree of its own", () => {
  show({ ...project, worktrees: [main, review] });
  const rows = [...tree().querySelectorAll(".tree-row")].map((r) => r.textContent);
  expect(rows).toEqual(["shop", "main", "review-fix-login"]);
  expect(tree().querySelector(".review-badge")).toBeNull();
});

test("the menu offers a review of a worktree on a branch, and a review's own actions", () => {
  show();
  rightClick("fix-login");
  expect(items()).toEqual([
    "New terminal here",
    "Copy path",
    "Show in Explorer",
    "Review in a new worktree…",
    "Rename…",
    "Delete…",
  ]);
  fireEvent.click(item("Review in a new worktree…"));
  expect(useHive.getState()).toMatchObject({ modal: "review-worktree", modalWorktree: login.id });
  act(() => openModal(null));

  rightClick("feat-checkout");
  expect(item("Review in a new worktree…").disabled).toBe(true);
  expect(item("Review in a new worktree…").title).toBe(
    "A detached worktree has no branch to review",
  );

  rightClick("review-fix-login");
  expect(items()).toEqual([
    "New terminal here",
    "Copy path",
    "Show in Explorer",
    "Review in a new worktree…",
    "Bring changes back…",
    "Discard review…",
    "Rename…",
    "Delete…",
  ]);
  fireEvent.click(item("Bring changes back…"));
  expect(useHive.getState()).toMatchObject({ modal: "bring-back", modalWorktree: reviewPath });
  act(() => openModal(null));
  rightClick("review-fix-login");
  fireEvent.click(item("Discard review…"));
  expect(useHive.getState()).toMatchObject({ modal: "discard-review", modalWorktree: reviewPath });
});

test("a review is created by the service and selected; its notes keep the dialog open", () => {
  const create = spyOn(transport, "createReview").mockResolvedValue();
  show();
  act(() => openModal("review-worktree", null, login.id));
  const dialog = screen.getByRole("dialog", { name: "Review in a new worktree" });
  expect(dialog.textContent).toContain("from the tip of worktree-fix-login");
  expect(dialog.textContent).toContain("Only committed work is in it.");
  const submit = within(dialog).getByRole("button", { name: "Create review" });
  expect(document.activeElement).toBe(submit);
  fireEvent.click(submit);
  expect(create).toHaveBeenLastCalledWith(login.path);

  // Another worktree's failure is not this one's.
  act(() => apply({ type: "create_worktree_failed", project: shop.id, name: "/x", message: "no" }));
  expect(within(dialog).queryByRole("alert")).toBeNull();
  const message = "review-fix-login and 100 more names are taken";
  act(() => apply({ type: "create_worktree_failed", project: shop.id, name: login.path, message }));
  expect(within(dialog).getByRole("alert").textContent).toBe(message);

  const notes = ["the review link could not be saved: denied"];
  act(() => apply({ type: "worktree_created", project, path: reviewPath, notes }));
  expect(useHive.getState().selection).toBe(reviewPath);
  expect(dialog.textContent).toContain(`Created ${reviewPath}`);
  expect(dialog.textContent).toContain(notes[0]);
  expect(within(dialog).queryByRole("button", { name: "Cancel" })).toBeNull();
  fireEvent.click(within(dialog).getByRole("button", { name: "Done" }));
  expect(useHive.getState().modal).toBeNull();
  expect(create).toHaveBeenCalledTimes(1);

  // Without notes it closes at once; Cancel sends nothing.
  act(() => openModal("review-worktree", null, main.id));
  act(() => apply({ type: "worktree_created", project, path: reviewPath, notes: [] }));
  expect(useHive.getState().modal).toBeNull();
  act(() => openModal("review-worktree", null, main.id));
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  expect(create).toHaveBeenCalledTimes(1);
  create.mockRestore();
});

const plan = (extra: Partial<ReviewPlan> = {}): ReviewPlan => ({
  reviewed: login.path,
  into: "worktree-fix-login",
  base: "77aa000",
  branch: "review-fix-login",
  tip: "9f1c2ab0000",
  commits: MOCK_REVIEW_COMMITS,
  total: 2,
  fast_forward: true,
  ...extra,
});

test("bringing changes back shows the commits first and merges only on the click", () => {
  const ask = spyOn(transport, "planBringBack").mockResolvedValue();
  const bring = spyOn(transport, "bringBack").mockResolvedValue();
  show();
  act(() => openModal("bring-back", null, reviewPath));
  expect(ask).toHaveBeenLastCalledWith(reviewPath);
  const dialog = screen.getByRole("dialog", { name: "Bring changes back" });
  const submit = within(dialog).getByRole("button", { name: "Bring back" }) as HTMLButtonElement;
  expect(dialog.textContent).toContain("Looking at the review's commits…");
  expect(submit.disabled).toBe(true);
  fireEvent.submit(submit);
  expect(bring).not.toHaveBeenCalled();

  // Another review's plan is not this one's.
  act(() => apply({ type: "bring_back_plan", path: "/x", plan: plan(), error: null }));
  expect(dialog.textContent).toContain("Looking");
  act(() => apply({ type: "bring_back_plan", path: reviewPath, plan: plan(), error: null }));
  expect(dialog.textContent).toContain(
    "2 commits of review-fix-login go into worktree-fix-login, as a fast-forward:",
  );
  const listed = [...dialog.querySelectorAll(".review-commits li")].map((li) => li.textContent);
  expect(listed).toEqual([
    "9f1c2abKeep the redirect after a login",
    "4d7e013Test the expired session",
  ]);
  expect(dialog.textContent).not.toContain("more");
  expect(submit.disabled).toBe(false);
  fireEvent.click(submit);
  expect(bring).toHaveBeenLastCalledWith(
    reviewPath,
    "worktree-fix-login",
    "77aa000",
    "9f1c2ab0000",
  );

  // git's message, line by line; another review's failure is not this one's.
  act(() => apply({ type: "bring_back_failed", path: "/x", message: "no" }));
  expect(within(dialog).queryByRole("alert")).toBeNull();
  const conflict =
    "git merge stopped; nothing was changed:\nCONFLICT (content): Merge conflict in a";
  act(() => apply({ type: "bring_back_failed", path: reviewPath, message: conflict }));
  const alert = within(dialog).getByRole("alert");
  expect(alert.textContent).toBe(conflict);
  expect(alert.className).toBe("field-error said");

  // Brought back: the dialog closes with a notice.
  const done = { type: "brought_back", project, path: reviewPath, commits: 2 } as const;
  act(() => apply({ ...done, fast_forward: true }));
  expect(useHive.getState().modal).toBeNull();
  expect([notice(), noticeKind()]).toEqual([
    "Brought 2 commits back into worktree-fix-login (fast-forward).",
    "info",
  ]);
  ask.mockRestore();
  bring.mockRestore();
});

test("a plan with more commits than listed, a merge commit, nothing to bring, or none at all", () => {
  spyOn(transport, "planBringBack").mockResolvedValue();
  show();
  act(() => openModal("bring-back", null, reviewPath));
  const dialog = screen.getByRole("dialog", { name: "Bring changes back" });
  const submit = within(dialog).getByRole("button", { name: "Bring back" }) as HTMLButtonElement;
  const answer = (p: ReviewPlan | null, error: string | null = null) =>
    act(() => apply({ type: "bring_back_plan", path: reviewPath, plan: p, error }));

  answer(plan({ commits: MOCK_REVIEW_COMMITS.slice(0, 1), total: 1, fast_forward: false }));
  expect(dialog.textContent).toContain(
    "1 commit of review-fix-login go into worktree-fix-login, with a merge commit:",
  );
  answer(plan({ total: 3 }));
  expect(dialog.textContent).toContain("and 1 commit more");
  answer(plan({ total: 102 }));
  expect(dialog.textContent).toContain("and 100 commits more");

  answer(plan({ commits: [], total: 0 }));
  expect(dialog.textContent).toContain(
    "Nothing to bring back: every commit of review-fix-login is on worktree-fix-login.",
  );
  expect(submit.disabled).toBe(true);
  answer(null, "review-fix-login is not a review");
  expect(within(dialog).getByRole("alert").textContent).toBe("review-fix-login is not a review");
  expect(submit.disabled).toBe(true);

  // Brought back elsewhere (another review), this dialog stays; no plan: no branch named.
  act(() => openModal("remove-worktree", null, login.id));
  act(() =>
    apply({ type: "brought_back", project, path: reviewPath, commits: 1, fast_forward: false }),
  );
  expect(useHive.getState().modal).toBe("remove-worktree");
  expect(notice()).toBe("Brought 1 commit back (merge commit).");
});

test("discarding a review asks first; a refusal offers to force it", () => {
  const discard = spyOn(transport, "discardReview").mockResolvedValue();
  show();
  select(reviewPath);
  act(() => openModal("discard-review", null, reviewPath));
  const dialog = screen.getByRole("dialog", { name: "Discard review" });
  expect(dialog.textContent).toContain(
    `Delete the review review-fix-login and its folder ${reviewPath}?`,
  );
  expect(dialog.textContent).toContain(
    "Its branch review-fix-login is deleted too if every commit",
  );
  const submit = within(dialog).getByRole("button", { name: "Discard" });
  expect(document.activeElement).toBe(submit);
  fireEvent.click(submit);
  expect(discard).toHaveBeenLastCalledWith(reviewPath, false);
  const message = "in use by fish (1): close its terminals first";
  act(() => apply({ type: "remove_worktree_failed", path: reviewPath, message }));
  expect(within(dialog).getByRole("alert").textContent).toBe(message);
  fireEvent.click(within(dialog).getByRole("button", { name: "Discard anyway" }));
  expect(discard).toHaveBeenLastCalledWith(reviewPath, true);
  const updated = { ...project, worktrees: [main, login, detached] };
  act(() => apply({ type: "worktree_removed", project: updated, path: reviewPath }));
  expect(useHive.getState().modal).toBeNull();
  expect(useHive.getState().selection).toBe(shop.id);

  // Detached, there is no branch to speak of; Cancel closes.
  const unbranched = { ...project, worktrees: [main, { ...review, branch: null }] };
  act(() => apply({ type: "projects", projects: [unbranched] }));
  act(() => openModal("discard-review", null, reviewPath));
  const again = screen.getByRole("dialog", { name: "Discard review" });
  expect(again.textContent).not.toContain("Its branch");
  fireEvent.click(within(again).getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  discard.mockRestore();
});

test("a review dialog for a worktree that is gone shows nothing", () => {
  show();
  for (const modal of ["review-worktree", "bring-back", "discard-review"] as const) {
    act(() => openModal(modal, null, "/gone"));
    expect(screen.queryByRole("dialog")).toBeNull();
  }
});
