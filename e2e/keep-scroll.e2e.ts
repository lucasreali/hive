import { expect, type Page, test } from "@playwright/test";

/** The text of the first line the shown file's view shows at its top. */
const firstLine = (page: Page) =>
  page.evaluate(() => {
    const scroller = document.querySelector(".file-view .cm-scroller");
    if (!scroller) return null;
    const top = scroller.getBoundingClientRect().top;
    const lines = [...scroller.querySelectorAll(".cm-line")];
    return lines.find((l) => l.getBoundingClientRect().bottom > top + 9)?.textContent ?? null;
  });

test("a file shown again keeps its scroll, whichever way it was left (15.4)", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  // A terminal in the worktree, to leave the file by.
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  const files = page.getByRole("region", { name: "Files" });
  const open = async (name: string, path = name) => {
    await files.getByRole("treeitem", { name, exact: true }).dblclick();
    await expect(page.getByRole("region", { name: path }).locator(".cm-content")).not.toBeEmpty();
  };
  await files.getByRole("treeitem", { name: /^src / }).click();
  await open("server.ts", "src/server.ts");
  // Far down, by the wheel as the human does.
  await page.locator(".file-view .cm-scroller").hover();
  await page.mouse.wheel(0, 4000);
  await expect.poll(() => firstLine(page)).not.toBe("// server line 1");
  const left = await firstLine(page);
  expect(left).toMatch(/server line (\d{3})/);
  const tab = page.getByRole("tab", { name: "server.ts" });

  // A terminal tab, and back.
  await page.getByRole("tab", { name: "fix-login" }).click();
  await tab.click();
  await expect.poll(() => firstLine(page)).toBe(left);
  // Another file, and back.
  await open("README.md");
  await tab.click();
  await expect.poll(() => firstLine(page)).toBe(left);
  // Another worktree, and back.
  await tree.getByRole("button", { name: "feat-checkout" }).click();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await tab.click();
  await expect.poll(() => firstLine(page)).toBe(left);
  // A narrower window meanwhile: the same line still at the top.
  await page.getByRole("tab", { name: "fix-login" }).click();
  await page.setViewportSize({ width: 1100, height: 700 });
  await tab.click();
  await expect.poll(() => firstLine(page)).toBe(left);
});
