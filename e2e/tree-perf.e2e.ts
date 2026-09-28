import { expect, test } from "@playwright/test";

// 9.23: ↑/↓ in the files tree of a big repository stays within a frame. The time is the main
// thread's work for one key: React's render and commit plus the layout it forces, measured up
// to the task after it, not to the next frame (waiting for it would add up to a frame).
const FILES = 50_000;
const KEYS = 200;
// Default pending human review: the 95th-percentile key within one 60 Hz frame.
const MAX_P95_KEY_MS = 16;

const percentile = (values: number[], p: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] ?? NaN;
};
const round = (n: number) => Math.round(n * 10) / 10;

test("files tree: ↑/↓ in a 50 000-file worktree stays under a frame", async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  const tree = panel.getByRole("tree", { name: "Files" });
  await expect(tree.getByRole("treeitem").first()).toBeVisible();

  // 50 folders of 20 folders of 50 files, the first ten top folders and their folders open:
  // 10 500 rows to move through.
  const rows = await page.evaluate(async (count) => {
    const url = "/src/store.ts";
    const { apply, useHive } = await import(/* @vite-ignore */ url);
    const path = "/home/user/projects/shop/.claude/worktrees/fix-login";
    const files = Array.from(
      { length: count },
      (_, i) => `dir${String(i % 50).padStart(2, "0")}/sub${i % 20}/file${i}.ts`,
    ).sort();
    const collapsed: Record<string, boolean> = {};
    for (let d = 0; d < 10; d++) {
      const dir = `dir${String(d).padStart(2, "0")}`;
      collapsed[`files:${path}/${dir}`] = false;
      for (let s = 0; s < 20; s++) collapsed[`files:${path}/${dir}/sub${s}`] = false;
    }
    apply({ type: "files", path, files, truncated: false });
    useHive.setState((s: { collapsed: Record<string, boolean> }) => ({
      collapsed: { ...s.collapsed, ...collapsed },
    }));
    await new Promise((resolve) => setTimeout(resolve, 500));
    return document.querySelector('[role="tree"][aria-label="Files"]')?.getAttribute("style");
  }, FILES);
  expect(rows).toContain("height");
  await expect(tree.getByRole("treeitem", { name: "dir00" })).toHaveAttribute(
    "aria-expanded",
    "true",
  );

  const times = await page.evaluate(async (keys) => {
    const tree = document.querySelector('[role="tree"][aria-label="Files"]') as HTMLElement;
    tree.focus();
    const out: number[] = [];
    // React renders a key's update in the microtask it queued while handling the key, so one
    // queued after the dispatch runs once it is done (a later task could wait behind others).
    const rendered = () => new Promise((resolve) => queueMicrotask(() => resolve(null)));
    for (let i = 0; i < keys; i++) {
      const key = i % 4 === 3 ? "ArrowUp" : "ArrowDown";
      const start = performance.now();
      tree.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      await rendered();
      tree.getBoundingClientRect();
      out.push(performance.now() - start);
      await new Promise((resolve) => requestAnimationFrame(resolve));
    }
    return out;
  }, KEYS);
  // The keys moved the active row: 150 down, 50 up.
  const active = await tree.getAttribute("aria-activedescendant");
  expect(active).toMatch(/-100$/);

  const results = {
    files: FILES,
    keys: times.length,
    keyMs: {
      p50: round(percentile(times, 50)),
      p95: round(percentile(times, 95)),
      max: round(Math.max(...times)),
    },
  };
  console.log(JSON.stringify(results));
  expect(results.keyMs.p95).toBeLessThan(MAX_P95_KEY_MS);
});
