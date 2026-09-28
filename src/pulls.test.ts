import { afterEach, expect, spyOn, test } from "bun:test";
import { actOnPull, hidePull, newPull, type Pulls, pullOf, setPullBusy, showPull } from "./pulls";
import { apply } from "./reduce";
import { initialState, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_PULLS } from "./transport/mockPulls";

afterEach(() => useHive.setState(initialState, true));

const SHOP = "/home/user/projects/shop";
const shop: Pulls = {
  project: SHOP,
  repo: null,
  mine: MOCK_PULLS[SHOP]?.mine ?? [],
  review: MOCK_PULLS[SHOP]?.review ?? [],
  fetched_ms: 1,
  error: null,
};

test("the lists are kept by project and name each worktree's pull request", () => {
  apply({ type: "pulls", ...shop });
  apply({ type: "pulls", ...shop, project: "/api", mine: [], review: [], error: "no remote" });
  const s = useHive.getState();
  expect(s.pulls[SHOP]).toEqual(shop);
  expect(s.pulls["/api"]?.error).toBe("no remote");
  expect(pullOf(s, `${SHOP}/.claude/worktrees/fix-login`)).toEqual([SHOP, shop.mine[0]]);
  expect(pullOf(s, "/elsewhere")).toEqual([]);
});

test("details show for the open pull request only; actions wait for their answer", () => {
  const open = spyOn(transport, "openPull").mockResolvedValue();
  const act = spyOn(transport, "actOnPull").mockResolvedValue();
  useHive.setState({ pullError: { project: SHOP, number: 1, message: "old" } });
  showPull(SHOP, 12);
  expect(open).toHaveBeenCalledWith(SHOP, 12);
  expect(useHive.getState().pullError).toBeNull();
  const detail = { type: "pull", project: SHOP, pull: null, error: "gone" } as const;
  apply({ ...detail, number: 13 });
  apply({ ...detail, project: "/api", number: 12 });
  expect(useHive.getState().openPull).toEqual({
    project: SHOP,
    number: 12,
    detail: null,
    error: null,
  });
  apply({ ...detail, number: 12 });
  expect(useHive.getState().openPull?.error).toBe("gone");

  actOnPull(SHOP, 12, { kind: "close" });
  expect(act).toHaveBeenCalledWith(SHOP, 12, { kind: "close" });
  expect(useHive.getState().pullBusy).toEqual({ project: SHOP, number: 12, action: "close" });
  const failed = { type: "pull_failed", project: SHOP, number: 12, message: "no" } as const;
  apply(failed);
  expect(useHive.getState().pullBusy).toBeNull();
  expect(useHive.getState().pullError).toEqual({ project: SHOP, number: 12, message: "no" });

  setPullBusy({ project: SHOP, number: null, action: "create" });
  newPull(SHOP, "/w");
  const s = useHive.getState();
  expect([s.modal, s.modalProject, s.modalWorktree, s.pullError]).toEqual([
    "new-pull",
    SHOP,
    "/w",
    null,
  ]);
  apply({ type: "pull_done", project: SHOP, number: 20, message: "Opened pull request #20" });
  const done = useHive.getState();
  expect([done.modal, done.pullBusy, done.notices.at(-1)]).toMatchObject([
    null,
    null,
    { kind: "info", text: "Opened pull request #20" },
  ]);
  // Another dialog stays.
  useHive.setState({ modal: "settings" });
  apply({ type: "pull_done", project: SHOP, number: 12, message: "Merged" });
  expect(useHive.getState().modal).toBe("settings");

  hidePull();
  expect(useHive.getState().openPull).toBeNull();
  open.mockRestore();
  act.mockRestore();
});
