import { expect, type Locator, test } from "@playwright/test";

/** The folder `name` of the files tree. */
const folder = (files: Locator, name: string) =>
  files.getByRole("treeitem").filter({ has: files.page().getByText(name, { exact: true }) });

/** A row name's colour. */
const color = (row: Locator) =>
  row.locator(".name").evaluate((name) => getComputedStyle(name).color);

test("ignored: what git ignores shows dimmed, an ignored folder lists what it holds only while open, a .env is edited and saved (14.2)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const files = page.getByRole("region", { name: "Files" });
  const env = files.getByRole("treeitem", { name: ".env" });
  const modules = folder(files, "node_modules");
  await expect(env).toHaveAttribute("data-ignored", "true");
  await expect(modules).toHaveAttribute("data-ignored", "true");
  await expect(modules).toHaveAttribute("aria-expanded", "false");
  // Dimmed: a muted name, not the text colour of a listed file.
  const readme = files.getByRole("treeitem", { name: "README.md" });
  await expect(readme).toHaveAttribute("data-ignored", "false");
  expect(await color(env)).not.toBe(await color(readme));

  // Opened, the service lists one level of it, and a folder inside it the same way.
  const lock = files.getByRole("treeitem", { name: ".package-lock.json" });
  await expect(lock).toHaveCount(0);
  await modules.click();
  await expect(lock).toHaveAttribute("data-ignored", "true");
  await folder(files, "react").click();
  await expect(files.getByRole("treeitem", { name: "index.js" })).toBeVisible();
  await page.screenshot({ path: "target/e2e/ignored.png" });
  // Closed, what it held goes.
  await modules.click();
  await expect(lock).toHaveCount(0);
  await expect(files.getByRole("treeitem", { name: "index.js" })).toHaveCount(0);

  // An ignored file opens and saves as any other.
  await env.click();
  const view = page.getByRole("region", { name: ".env" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("API_KEY=dev\n");
  const unsaved = page.getByRole("button", { name: "Close file .env (unsaved changes)" });
  await expect(unsaved).toBeVisible();
  await page.keyboard.press("Control+s");
  await expect(unsaved).toBeHidden();
  await expect(view.locator(".cm-content")).toContainText("API_KEY=dev");
});
