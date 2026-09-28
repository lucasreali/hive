import type { PullAction, PullDetail, PullRepo, PullSummary } from "../pulls";
import type { ServiceMessage } from "../store";

const SHOP = "/home/user/projects/shop";
const REPO: PullRepo = {
  name: "user/shop",
  url: "https://github.com/user/shop",
  merge_methods: ["merge", "squash"],
  default_merge: "squash",
};

const pull = (number: number, title: string, extra: Partial<PullSummary> = {}): PullSummary => ({
  number,
  title,
  url: `${REPO.url}/pull/${number}`,
  state: "open",
  branch: `feature-${number}`,
  base: "main",
  author: "mock-work",
  review: null,
  checks: null,
  updated_at: "2026-09-27T09:00:00Z",
  worktree: null,
  ...extra,
});

/**
 * The fake service's pull requests (9.31), by project: shop has a GitHub repository, api none.
 * #14 asks for the account's review, and GitHub refuses to merge it (no approval yet).
 */
export const MOCK_PULLS: Record<string, { mine: PullSummary[]; review: PullSummary[] }> = {
  [SHOP]: {
    mine: [
      pull(12, "Fix the login redirect", {
        branch: "worktree-fix-login",
        worktree: `${SHOP}/.claude/worktrees/fix-login`,
        review: "approved",
        checks: "passing",
      }),
      pull(9, "New checkout flow", { state: "draft", checks: "running" }),
      pull(5, "Bump dependencies", { state: "merged", checks: "passing" }),
    ],
    review: [
      pull(14, "Rate limit the API", {
        author: "octo-reviewer",
        review: "review_required",
        checks: "failing",
      }),
    ],
  },
};

const detail = (summary: PullSummary): PullDetail => ({
  summary,
  body: `## What\n\n${summary.title}, as asked in [the issue](${REPO.url}/issues/1).\n\n- one\n- two`,
  head: "a".repeat(40),
  additions: 12,
  deletions: 3,
  conflicts: false,
  notes: [
    {
      author: "octo-reviewer",
      body: "Looks good, one nit.",
      at: summary.updated_at,
      review: "commented",
    },
  ],
  checks: [
    {
      name: "test",
      workflow: "CI",
      state: summary.checks ?? "skipped",
      url: `${REPO.url}/actions/runs/1`,
    },
  ],
  files: [{ path: "src/login.ts", additions: 12, deletions: 3 }],
});

/**
 * The pull requests side of the mock transport: lists, details and actions as the service
 * answers them (`later` sends a message). `holder` names the project of a worktree.
 */
export function createMockPulls(
  later: (message: ServiceMessage) => void,
  holder: (worktree: string) => string | null,
) {
  const lists = structuredClone(MOCK_PULLS);
  const find = (project: string, number: number) =>
    [...(lists[project]?.mine ?? []), ...(lists[project]?.review ?? [])].find(
      (p) => p.number === number,
    );
  const sendList = (project: string) => {
    const list = lists[project];
    later({
      type: "pulls",
      project,
      repo: list ? REPO : null,
      mine: structuredClone(list?.mine ?? []),
      review: structuredClone(list?.review ?? []),
      fetched_ms: Date.now(),
      error: list ? null : "No remote of this repository is on github.com",
    });
  };
  const sendPull = (project: string, number: number) => {
    const found = find(project, number);
    const pull = found ? detail(structuredClone(found)) : null;
    const error = found ? null : `no pull request #${number}`;
    later({ type: "pull", project, number, pull, error });
  };
  return {
    async listPulls(project: string, _force: boolean) {
      sendList(project);
    },
    async openPull(project: string, number: number) {
      sendPull(project, number);
    },
    /** Every action but a checkout (a new worktree, made by the transport). */
    act(project: string, number: number, action: Exclude<PullAction, { kind: "checkout" }>) {
      const found = find(project, number);
      if (!found || number === 14) {
        const message = `gh pr ${action.kind} failed: GraphQL: At least 1 approving review is required by reviewers with write access. (mergePullRequest)`;
        return later({ type: "pull_failed", project, number, message });
      }
      const [state, message] = {
        ready: ["open", `#${number} is ready for review`],
        close: ["closed", `Closed pull request #${number}`],
        merge: ["merged", `Merged pull request #${number}`],
      }[action.kind] as [PullSummary["state"], string];
      found.state = state;
      later({ type: "pull_done", project, number, message });
      sendPull(project, number);
      sendList(project);
    },
    async createPull(worktree: string, title: string, _body: string, base: string, draft: boolean) {
      const project = holder(worktree) ?? "";
      const list = lists[project];
      if (!list || !title.trim()) {
        const message = list
          ? "A title of 1 to 256 characters is needed"
          : "No remote of this repository is on github.com";
        return later({ type: "pull_failed", project, number: null, message });
      }
      const number = 20 + list.mine.length;
      const state = draft ? "draft" : "open";
      list.mine.unshift(pull(number, title.trim(), { base, state, worktree }));
      later({ type: "pull_done", project, number, message: `Opened pull request #${number}` });
      sendList(project);
    },
  };
}
