import { expect, test } from "@playwright/test";

test("a folder of repositories is a group: its agents, then its projects one level in", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const rows = () => tree.locator(".tree-row").allTextContents();

  // Added as any project: the fake service follows it as a group of its two repositories.
  await page.getByTitle("Add project (Ctrl+Shift+O)").click();
  const dialog = page.getByRole("dialog", { name: "Add project" });
  const field = dialog.getByLabel("Folder", { exact: true });
  await field.fill("/home/user/work");
  // Enter adds once the typed folder is listed.
  await expect(dialog.getByRole("button", { name: /^Add project/ })).toBeEnabled();
  await field.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(tree.getByRole("button", { name: "backend", exact: true })).toBeVisible();
  expect((await rows()).slice(-5)).toEqual(["work", "backend", "main", "frontend", "main"]);

  // Each level one step in: group, its projects, their worktrees.
  const iconX = async (name: string, kind: string) =>
    (
      await tree
        .locator(`.tree-row.${kind}`, { hasText: new RegExp(`^${name}`) })
        .first()
        .locator(".row-main > svg")
        .boundingBox()
    )?.x ?? 0;
  const [shop, work, backend] = await Promise.all([
    iconX("shop", "project"),
    iconX("work", "project"),
    iconX("backend", "project"),
  ]);
  expect(work).toBe(shop);
  expect(backend).toBeGreaterThan(work);
  await page.screenshot({ path: "target/e2e/group.png" });

  // A terminal on the group runs in its folder; its agent shows under the group row, before its
  // projects.
  await tree.getByRole("button", { name: "work", exact: true }).click();
  await page.getByRole("button", { name: "New terminal", exact: true }).click();
  await expect(page.getByRole("tablist").getByRole("tab", { name: "work" })).toBeVisible();
  await page.keyboard.type("claude");
  await page.keyboard.press("Enter");
  await expect(tree.locator(".tree-row.agent")).toHaveCount(1);
  expect((await rows()).slice(-6)[0]).toBe("work");
  expect((await rows()).slice(-6)[1]).toMatch(/^idleClaudeidle\ds$/);

  // Collapsed, the group shows the most urgent state inside it.
  await page.keyboard.type("state waiting_permission");
  await page.keyboard.press("Enter");
  await tree.getByRole("button", { name: "Collapse work" }).click();
  await expect(tree.getByRole("button", { name: "backend", exact: true })).toHaveCount(0);
  const rollup = tree.locator(".tree-row.group .state-icon");
  await expect(rollup).toHaveAttribute("data-state", "waiting_permission");
  await page.screenshot({ path: "target/e2e/group-collapsed.png" });
});
