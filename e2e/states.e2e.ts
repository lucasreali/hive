import { expect, test } from "@playwright/test";

test("states: every agent and subagent shows the state icon the service sent", async ({ page }) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const rows = tree.locator(".tree-row.agent, .tree-row.subagent");
  await expect(rows).toHaveCount(11);
  const shown = await rows.evaluateAll((els) =>
    els.map((el) => {
      const icon = el.querySelector(".state-icon") as SVGElement;
      const box = icon.getBoundingClientRect();
      return [
        el.className,
        icon.getAttribute("aria-label"),
        el.querySelector(".label")?.textContent,
        box.width,
        box.height,
      ];
    }),
  );
  expect(shown).toEqual([
    ["tree-row agent", "running subagents", "Claude", 14, 14],
    ["tree-row subagent", "working", "subagent: Explore", 14, 14],
    ["tree-row subagent", "working", "subagent: unknown", 14, 14],
    ["tree-row agent", "waiting for permission", "Claude", 14, 14],
    ["tree-row subagent", "waiting for permission", "subagent: general-purpose", 14, 14],
    ["tree-row subagent", "idle", "subagent: Explore", 14, 14],
    ["tree-row agent", "waiting for you", "Claude", 14, 14],
    ["tree-row agent", "error", "Claude", 14, 14],
    ["tree-row agent", "ended", "Claude", 14, 14],
    ["tree-row agent", "working", "Claude", 14, 14],
    ["tree-row agent", "idle", "Claude", 14, 14],
  ]);
  // Colors come from the state tokens; only alerting states color the agent's state name.
  const permission = tree.locator(".tree-row.agent .state-label[data-state=waiting_permission]");
  await expect(permission).toHaveCSS("color", "rgb(222, 193, 132)");
  const working = tree.locator(".tree-row.agent .state-label[data-state=working]");
  await expect(working).toHaveCSS("color", "rgb(169, 175, 188)");
  // Working spins, in its state's color.
  const spinning = tree.locator(".tree-row.agent .state-icon[data-state=working]");
  await expect(spinning).toHaveCSS("animation-name", "hive-spin");
  await expect(spinning).toHaveCSS("color", "rgb(116, 173, 232)");
});

test("states: collapsed nodes show the most urgent state inside; F8 walks the pending agents", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const chip = page.getByRole("button", { name: "3 pending: go to the next (F8)" });
  await expect(chip).toBeVisible();
  await expect(chip).toHaveCSS("color", "rgb(222, 193, 132)");

  // Collapsed: the project shows waiting for permission (a subagent's, via its agent).
  await tree.getByRole("button", { name: "Collapse shop" }).click();
  const shop = tree.locator(".tree-row.project").first();
  const icon = shop.locator(".state-icon");
  await expect(icon).toHaveAttribute("data-state", "waiting_permission");
  expect(await icon.boundingBox()).toMatchObject({ width: 12, height: 12 });
  await tree.getByRole("button", { name: "Collapse refactor-auth" }).click();
  const refactor = tree.locator(".tree-row.worktree", { hasText: "refactor-auth" });
  await expect(refactor.locator(".state-icon")).toHaveAttribute("data-state", "working");

  // F8 opens the collapsed project and selects each pending agent in tree order, wrapping.
  const selected = tree.locator(".tree-row.agent[data-selected=true] .state-label");
  const expected = ["waiting for permission", "waiting for you", "error", "waiting for permission"];
  for (const label of expected) {
    await page.keyboard.press("F8");
    await expect(selected).toHaveText(label);
  }
  await expect(icon).toHaveCount(0);
  // Clicking the counter is F8.
  await chip.click();
  await expect(selected).toHaveText("waiting for you");
});

test("states: a subagent's own worktree is its parent row, not at project level", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const own = tree.locator(".tree-row.own-worktree");
  await expect(own).toHaveCount(1);
  await expect(own).toHaveText("tests-login");
  await expect(own).toHaveAttribute(
    "title",
    "/home/user/projects/shop/.claude/worktrees/tests-login",
  );
  const owned = own.locator("xpath=following-sibling::ul[1]").locator(".tree-row.subagent");
  await expect(owned).toHaveCount(1);
  await expect(owned).toHaveText(/subagent: general-purpose/);
  await expect(tree.locator(".tree-row.worktree", { hasText: "tests-login" })).toHaveCount(0);
  // 22px, at the subagents' indent; its subagent one step (16px) further in. The agent's tree
  // line goes on past both to the next subagent.
  const [height, padding, line] = await own.evaluate((el) => [
    el.getBoundingClientRect().height,
    getComputedStyle(el).paddingLeft,
    getComputedStyle(el, "::before").height,
  ]);
  expect([height, padding, line]).toEqual([22, "56px", "22px"]);
  const [subPadding, through] = await owned.evaluate((el) => [
    getComputedStyle(el).paddingLeft,
    getComputedStyle(el.closest("ul") as Element, "::before").left,
  ]);
  expect([subPadding, through]).toEqual(["72px", "45px"]);
});
