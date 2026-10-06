import { expect, type Page, test } from "@playwright/test";

const cssVar = (page: Page, name: string) =>
  page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name), name);

/** The colour `token` resolves to, as the browser computes a background with it. */
const color = (page: Page, token: string) =>
  page.evaluate((token) => {
    const probe = document.createElement("div");
    probe.style.backgroundColor = `var(${token})`;
    document.body.append(probe);
    const value = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return value;
  }, token);

test("the editor marks added, modified and deleted lines in its gutter, in dark and light (14.4)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  for (const name of ["src", "auth"]) {
    await panel
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await panel.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  await view.getByRole("button", { name: "Edit", exact: true }).click();
  const gutter = view.locator(".cm-changeGutter");
  // Against HEAD the file has modified lines (line 2, and 39–41).
  await expect(gutter.locator(".cm-change-modified")).toHaveCount(4);

  // A new first line, then line 10 removed: typed, not saved.
  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// new\n");
  for (let i = 0; i < 8; i++) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Delete");
  const added = gutter.locator(".cm-change-added");
  const modified = gutter.locator(".cm-change-modified");
  const deleted = gutter.locator(".cm-change-deleted");
  await expect(added).toHaveCount(1);
  await expect(modified).toHaveCount(4);
  await expect(deleted).toHaveCount(1);
  // On the cursor's line (14.3): `check` below sees the marker drawn over the line's tint.
  await expect(deleted).toHaveClass(/cm-activeLineGutter/);
  // Their own gutter, left of the line numbers.
  const numbers = await view.locator(".cm-lineNumbers").boundingBox();
  expect((await gutter.boundingBox())?.x).toBeLessThan(numbers?.x ?? 0);

  const check = async () => {
    await expect(added).toHaveCSS("background-color", await color(page, "--git-added"));
    await expect(modified.first()).toHaveCSS(
      "background-color",
      await color(page, "--git-modified"),
    );
    await expect(deleted).toHaveCSS("background-image", /linear-gradient/);
    const wedge = await deleted.evaluate((e) => getComputedStyle(e).backgroundImage);
    expect(wedge).toContain(await color(page, "--git-deleted"));
  };
  expect(await cssVar(page, "--bg")).toBe("#282c33");
  await check();
  await page.screenshot({ path: "target/e2e/gutter-markers-dark.png" });

  await page.evaluate(() => {
    document.documentElement.dataset.theme = "one-light";
  });
  await expect.poll(() => cssVar(page, "--bg")).toBe("#fafafa");
  await check();
  await page.screenshot({ path: "target/e2e/gutter-markers-light.png" });
});
