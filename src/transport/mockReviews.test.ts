import { expect, test } from "bun:test";
import type { ServiceMessage } from "../protocol";
import { createMockTransport, MOCK_REPOS, MOCK_REVIEW_COMMITS, mockReviewName } from "./mock";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("a review's name is the service's", () => {
  expect(mockReviewName("fix-login")).toBe("review-fix-login");
  expect(mockReviewName("Feature/Login.Page-")).toBe("review-feature-login-page");
});

test("reviews (27.4): made, planned, brought back once and discarded as the service does", async () => {
  const [shop, api] = MOCK_REPOS;
  const detached = { ...shop.worktrees[2], branch: null };
  const repos = [{ ...shop, worktrees: [...shop.worktrees.slice(0, 2), detached] }, api];
  const transport = createMockTransport(null, null, { repos });
  const messages: ServiceMessage[] = [];
  await transport.connect((m) => messages.push(m));
  await tick();
  const wt = (name: string) => `${shop.path}/.claude/worktrees/${name}`;
  const answer = async (request: () => Promise<void>) => {
    messages.length = 0;
    await request();
    await tick();
    return messages[0] as ServiceMessage;
  };
  const said = async (request: () => Promise<void>) => {
    const m = await answer(request);
    return "message" in m ? m.message : m.type;
  };

  expect(await said(() => transport.createReview("/nope"))).toBe(
    "/nope is not a worktree of a followed project",
  );
  expect(await said(() => transport.createReview(detached.path))).toBe(
    "feat-checkout is detached: there is no branch to review",
  );
  const made = await answer(() => transport.createReview(shop.path));
  if (made.type !== "worktree_created") throw new Error("expected a review");
  expect(made.path).toBe(wt("review-main"));
  const review = made.project.worktrees.find((w) => w.path === made.path);
  expect(review).toMatchObject({ branch: "review-main", review_of: shop.path, claude: true });
  expect(review?.status).toMatchObject({ ahead: 2, merged: false });
  expect(await answer(() => transport.createReview(shop.path))).toMatchObject({
    path: wt("review-main-2"),
  });
  expect(await answer(() => transport.createReview(wt("fix-login")))).toMatchObject({
    path: wt("review-fix-login"),
  });

  expect(await answer(() => transport.planBringBack(shop.path))).toEqual({
    type: "bring_back_plan",
    path: shop.path,
    plan: null,
    error: "main is not a review",
  });
  expect(await answer(() => transport.planBringBack("/nope"))).toMatchObject({
    error: "/nope is not a review",
  });
  const tip = MOCK_REVIEW_COMMITS[0]?.id as string;
  expect(await answer(() => transport.planBringBack(wt("review-main")))).toEqual({
    type: "bring_back_plan",
    path: wt("review-main"),
    plan: {
      reviewed: shop.path,
      into: "main",
      branch: "review-main",
      tip,
      commits: MOCK_REVIEW_COMMITS,
      total: 2,
      fast_forward: false,
    },
    error: null,
  });
  expect(await answer(() => transport.planBringBack(wt("review-fix-login")))).toMatchObject({
    plan: { into: "worktree-fix-login", fast_forward: true },
  });

  expect(await said(() => transport.bringBack("/nope", tip))).toBe("/nope is not a review");
  expect(await said(() => transport.bringBack(wt("review-main"), "0"))).toBe(
    "review-main changed since its commits were shown: look again",
  );
  // fix-login has changes to tracked files in the fake service.
  expect(await said(() => transport.bringBack(wt("review-fix-login"), tip))).toBe(
    "fix-login has uncommitted changes: commit or discard them first",
  );
  expect(await answer(() => transport.bringBack(wt("review-main"), tip))).toMatchObject({
    type: "brought_back",
    path: wt("review-main"),
    commits: 2,
    fast_forward: false,
  });
  expect(await said(() => transport.bringBack(wt("review-main"), tip))).toBe(
    "nothing to bring back: every commit of review-main is on main",
  );
  expect(await answer(() => transport.planBringBack(wt("review-main")))).toMatchObject({
    plan: { tip: "0000000", total: 0 },
  });

  expect(await said(() => transport.discardReview(wt("fix-login"), false))).toBe(
    "fix-login is not a review",
  );
  expect(await said(() => transport.discardReview("/nope", false))).toBe("/nope is not a review");
  expect(await said(() => transport.discardReview(wt("review-main"), false))).toBe(
    "worktree_removed",
  );
  // A reviewed worktree that goes leaves its review an ordinary worktree.
  const gone = await answer(() => transport.removeWorktree(wt("fix-login"), true));
  if (gone.type !== "worktree_removed") throw new Error("expected a removal");
  const left = gone.project.worktrees.find((w) => w.path === wt("review-fix-login"));
  expect(left?.review_of).toBeNull();
  const other = gone.project.worktrees.find((w) => w.path === wt("review-main-2"));
  expect(other?.review_of).toBe(shop.path);
});
