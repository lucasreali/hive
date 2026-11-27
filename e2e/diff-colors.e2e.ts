import { expect, type Locator, type Page, test } from "@playwright/test";
import { mod } from "./host";

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

const cssVar = (page: Page, name: string) =>
  page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name), name);

async function pick(page: Page, label: string, option: string) {
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Appearance" }).click();
  await dialog.getByRole("combobox", { name: label }).click();
  await page.getByRole("option", { name: option }).click();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
}

/** The diff view, the Diff panel's rows and counts and the gutter take the diff's colours. */
async function check(page: Page, panel: Locator, view: Locator) {
  const row = panel.getByRole("treeitem", { name: /session\.ts/ });
  await expect(row.locator(".status-letter")).toHaveCSS(
    "color",
    await color(page, "--diff-modified"),
  );
  await expect(panel.locator(".count-added").first()).toHaveCSS(
    "color",
    await color(page, "--diff-added"),
  );
  await expect(panel.locator(".count-removed").first()).toHaveCSS(
    "color",
    await color(page, "--diff-removed"),
  );
  await expect(view.locator(".cm-changedLine").first()).toHaveCSS(
    "background-color",
    await color(page, "--diff-added-line"),
  );
  await expect(view.locator(".cm-deletedChunk").first()).toHaveCSS(
    "background-color",
    await color(page, "--diff-removed-line"),
  );
  await expect(view.locator(".cm-changedLineGutter").first()).toHaveCSS(
    "background-color",
    await color(page, "--diff-added"),
  );
}

test("the diff colours follow the setting, Standard or Color-blind, in One Dark and One Light (28.6)", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
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
  await expect(view.locator(".cm-changedLine").first()).toBeVisible();

  // Standard: the git status colours' hues, at AA contrast (28.24).
  const root = page.locator("html");
  await expect(root).toHaveAttribute("data-diff-colors", "standard");
  expect(await cssVar(page, "--diff-added")).toBe("#a4c384");
  expect(await cssVar(page, "--diff-removed")).toBe("#f5a3a7");
  await check(page, panel, view);
  await page.screenshot({ path: "target/e2e/diff-colors-standard-dark.png" });

  await pick(page, "Diff colors", "Color-blind (blue and orange)");
  await expect(root).toHaveAttribute("data-diff-colors", "color-blind");
  await expect.poll(() => cssVar(page, "--diff-added")).toBe("#85beff");
  expect(await cssVar(page, "--diff-removed")).toBe("#f7ad6e");
  await check(page, panel, view);
  await page.screenshot({ path: "target/e2e/diff-colors-color-blind-dark.png" });

  await pick(page, "Theme", "One Light");
  await expect.poll(() => cssVar(page, "--bg")).toBe("#f0f0ee");
  expect(await cssVar(page, "--diff-added")).toBe("#0b56b5");
  expect(await cssVar(page, "--diff-removed")).toBe("#943f00");
  await check(page, panel, view);
  await page.screenshot({ path: "target/e2e/diff-colors-color-blind-light.png" });

  await pick(page, "Diff colors", "Standard");
  await expect(root).toHaveAttribute("data-diff-colors", "standard");
  await expect.poll(() => cssVar(page, "--diff-added")).toBe("#406338");
  expect(await cssVar(page, "--diff-removed")).toBe("#a03728");
  await check(page, panel, view);
  await page.screenshot({ path: "target/e2e/diff-colors-standard-light.png" });
});
