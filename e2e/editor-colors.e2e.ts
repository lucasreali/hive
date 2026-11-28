import { expect, test } from "@playwright/test";

/** Each theme's code colours (28.25): a keyword, a string, a comment and the plain text. */
const THEMES = {
  "one-dark": {
    keyword: "rgb(198, 120, 221)",
    string: "rgb(152, 195, 121)",
    comment: "rgb(125, 135, 153)",
    plain: "rgb(171, 178, 191)",
  },
  "one-light": {
    keyword: "rgb(166, 38, 164)",
    string: "rgb(80, 161, 79)",
    comment: "rgb(160, 161, 167)",
    plain: "rgb(56, 58, 66)",
  },
};

test("the editor's code takes One Dark's colours in One Dark and One Light's in One Light (28.25)", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  const files = page.getByRole("region", { name: "Files" });
  for (const name of ["src", "auth"]) {
    await files
      .getByRole("treeitem")
      .filter({ has: page.getByText(name, { exact: true }) })
      .click();
  }
  await files.getByRole("treeitem", { name: /session\.ts/ }).click();
  const view = page.getByRole("region", { name: "src/auth/session.ts" });
  const content = view.locator(".cm-content");
  await expect(content).toContainText("REMEMBER_ME_TTL // 30 days");
  const token = (text: string) => content.locator("span", { hasText: text }).first();

  for (const [theme, colors] of Object.entries(THEMES)) {
    // The settings have loaded by now, so the theme set here stays.
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await expect(token("import")).toHaveCSS("color", colors.keyword);
    await expect(token("// 30 days")).toHaveCSS("color", colors.comment);
    await expect(content).toHaveCSS("color", colors.plain);
    await expect(content.locator("span", { hasText: /^"/ }).first()).toHaveCSS(
      "color",
      colors.string,
    );
    await page.screenshot({ path: `target/e2e/editor-colors-${theme}.png` });
  }
});
