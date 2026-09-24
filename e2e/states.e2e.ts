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
    ["tree-row agent", "running subagents", "Claude", 12, 12],
    ["tree-row subagent", "working", "subagent: Explore", 12, 12],
    ["tree-row subagent", "working", "subagent: unknown", 12, 12],
    ["tree-row agent", "waiting for permission", "Claude", 12, 12],
    ["tree-row subagent", "waiting for permission", "subagent: general-purpose", 12, 12],
    ["tree-row subagent", "idle", "subagent: Explore", 12, 12],
    ["tree-row agent", "waiting for you", "Claude", 12, 12],
    ["tree-row agent", "error", "Claude", 12, 12],
    ["tree-row agent", "ended", "Claude", 12, 12],
    ["tree-row agent", "working", "Claude", 12, 12],
    ["tree-row agent", "idle", "Claude", 12, 12],
  ]);
  // Colors come from the state tokens; only alerting states color the agent's state name.
  const permission = tree.locator(".tree-row.agent .state-label[data-state=waiting_permission]");
  await expect(permission).toHaveCSS("color", "rgb(222, 193, 132)");
  const working = tree.locator(".tree-row.agent .state-label[data-state=working]");
  await expect(working).toHaveCSS("color", "rgb(169, 175, 188)");
  // Working pulses.
  const pulse = tree.locator(".tree-row.agent .state-icon[data-state=working] .pulse");
  await expect(pulse).toHaveCSS("animation-name", "hive-pulse");
});

test("states: collapsed nodes show the most urgent state inside; F8 walks the pending agents", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const chip = tree.getByRole("button", { name: "3 pending F8" });
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
