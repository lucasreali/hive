import { expect, type Locator, test } from "@playwright/test";
import { mod } from "./host";

test("editor search: ⌘F (Ctrl+F off a Mac) finds the next and previous match (18.38)", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+Home`);
  await page.keyboard.type("foo one\nfoo two\n");
  await page.keyboard.press(`${mod}+Home`);

  await page.keyboard.press(`${mod}+f`);
  const field = view.locator(".cm-search input[name=search]");
  await expect(field).toBeFocused();
  // The terminal's search bar stays closed: the key was the editor's.
  await expect(page.locator(".terminal-search")).toHaveCount(0);
  // Typed, not filled: the panel takes the query on keyup.
  await page.keyboard.type("foo");
  await page.keyboard.press("Enter");
  const active = view.locator(".cm-activeLine");
  await expect(active).toHaveText("foo one");
  await page.keyboard.press("Enter");
  await expect(active).toHaveText("foo two");
  await page.keyboard.press("Shift+Enter");
  await expect(active).toHaveText("foo one");
  await expect(view.locator(".cm-searchMatch")).toHaveCount(2);
  await page.screenshot({ path: "target/e2e/editor-search.png" });

  await page.keyboard.press("Escape");
  await expect(view.locator(".cm-search")).toHaveCount(0);
});

const box = async (l: Locator) => {
  const b = await l.boundingBox();
  if (!b) throw new Error("not shown");
  return b;
};
const middle = async (l: Locator) => {
  const b = await box(l);
  return b.y + b.height / 2;
};

for (const theme of ["one-dark", "one-light"]) {
  test(`editor find bar: one line per row, centred, icons and toggles, in ${theme} (23.10)`, async ({
    page,
  }) => {
    await page.goto("/");
    await page
      .getByRole("navigation", { name: "Projects" })
      .getByRole("button", { name: "fix-login" })
      .click();
    await page
      .getByRole("region", { name: "Files" })
      .getByRole("treeitem", { name: "README.md" })
      .click();
    const view = page.getByRole("region", { name: "README.md" });
    await expect(view.locator(".cm-content")).toContainText("export const value = 1;");
    // The settings have loaded by now, so the theme set here stays.
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await view.locator(".cm-line").first().click();
    await page.keyboard.press(`${mod}+Home`);
    await page.keyboard.type("foo one\nfoo two\nFoo food\n");
    await page.keyboard.press(`${mod}+Home`);

    await page.keyboard.press(`${mod}+f`);
    const bar = view.getByRole("search", { name: "Find in the file" });
    const find = bar.getByRole("textbox", { name: "Find" });
    const replace = bar.getByRole("textbox", { name: "Replace" });
    await expect(find).toBeFocused();
    await page.keyboard.type("foo");
    const matches = view.locator(".cm-searchMatch");
    await expect(matches).toHaveCount(4);

    // Layout: each row one line, its items centred on it; both fields one width.
    const rows = bar.locator(".find-row");
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) {
      const centre = await middle(row);
      for (const item of await row.locator(":scope > :not(.find-gap)").all()) {
        expect(Math.abs((await middle(item)) - centre)).toBeLessThanOrEqual(1);
      }
    }
    const fields = bar.locator(".find-field");
    expect((await box(fields.nth(0))).width).toBe((await box(fields.nth(1))).width);
    // The close button: centred on the Find row, as far from the right edge as the header's
    // actions above it.
    const close = bar.getByRole("button", { name: "Close search" });
    expect(Math.abs((await middle(close)) - (await middle(fields.nth(0))))).toBeLessThanOrEqual(1);
    const header = page.locator(".file-view-bar");
    const gap = async (outer: Locator, inner: Locator) => {
      const [o, i] = [await box(outer), await box(inner)];
      return o.x + o.width - (i.x + i.width);
    };
    const headerGap = await gap(header, header.locator("button").last());
    expect(Math.abs((await gap(bar, close)) - headerGap)).toBeLessThanOrEqual(1);
    await page.screenshot({ path: `target/e2e/find-bar-${theme}.png` });
    await bar.screenshot({ path: `target/e2e/find-bar-${theme}-bar.png` });

    // Next and previous, by their arrows.
    const active = view.locator(".cm-activeLine");
    const next = bar.getByRole("button", { name: "Next match" });
    await next.click();
    await expect(active).toHaveText("foo one");
    await next.click();
    await expect(active).toHaveText("foo two");
    await bar.getByRole("button", { name: "Previous match" }).click();
    await expect(active).toHaveText("foo one");

    // The toggles: pressed while on, each narrowing the matches.
    const toggle = (name: string) => bar.getByRole("button", { name });
    await toggle("Match case").click();
    await expect(toggle("Match case")).toHaveAttribute("aria-pressed", "true");
    await expect(matches).toHaveCount(3);
    await toggle("Match whole word").click();
    await expect(toggle("Match whole word")).toHaveAttribute("aria-pressed", "true");
    await expect(matches).toHaveCount(2);
    await bar.screenshot({ path: `target/e2e/find-bar-${theme}-pressed.png` });
    await toggle("Match case").click();
    await expect(toggle("Match case")).toHaveAttribute("aria-pressed", "false");
    // "Foo" counts again; "food" is not the whole word.
    await expect(matches).toHaveCount(3);
    await toggle("Match whole word").click();
    await find.fill("fo+d");
    await expect(matches).toHaveCount(0);
    await toggle("Use regular expression").click();
    await expect(toggle("Use regular expression")).toHaveAttribute("aria-pressed", "true");
    await expect(matches).toHaveCount(1);
    await toggle("Use regular expression").click();

    // Replace the current match, then every one.
    await find.fill("foo");
    await replace.fill("bar");
    await expect(matches).toHaveCount(4);
    await bar.getByRole("button", { name: "Replace", exact: true }).click();
    await expect(matches).toHaveCount(3);
    await bar.getByRole("button", { name: "Replace all" }).click();
    await expect(matches).toHaveCount(0);
    const line = (n: number) => view.locator(".cm-line").nth(n);
    await expect(line(0)).toHaveText("bar one");
    await expect(line(1)).toHaveText("bar two");
    await expect(line(2)).toHaveText("bar bard");

    // Esc in the bar closes it; so does its close button.
    await find.focus();
    await page.keyboard.press("Escape");
    await expect(bar).toHaveCount(0);
    await line(3).click();
    await page.keyboard.press(`${mod}+f`);
    await expect(find).toBeFocused();
    await close.click();
    await expect(bar).toHaveCount(0);
  });
}
