import { expect, type Locator, type Page, test } from "@playwright/test";

/** The colour on screen at the middle of a 1×1 px point, as [r, g, b]. */
async function pixel(page: Page, x: number, y: number): Promise<number[]> {
  const png = await page.screenshot({ clip: { x, y, width: 1, height: 1 } });
  return page.evaluate(
    async (src) => {
      const img = new Image();
      img.src = src;
      await img.decode();
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const ctx = canvas.getContext("2d");
      ctx?.drawImage(img, 0, 0);
      return [...(ctx?.getImageData(0, 0, 1, 1).data.slice(0, 3) ?? [])];
    },
    `data:image/png;base64,${png.toString("base64")}`,
  );
}

const distance = (a: number[], b: number[]) => a.reduce((sum, v, i) => sum + Math.abs(v - b[i]), 0);

/** A point in the empty space at the right end of `line`. */
async function rightOf(line: Locator) {
  const box = await line.boundingBox();
  if (!box) throw new Error("line not shown");
  return { x: box.x + box.width - 4, y: box.y + box.height / 2 };
}

for (const theme of ["one-dark", "one-light"]) {
  test(`active line: the cursor's line is highlighted in ${theme}, a selection on it still shows (14.3)`, async ({
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
    const bg = () =>
      page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--bg"));
    expect(await bg()).toBe(theme === "one-light" ? "#fafafa" : "#282c33");

    const lines = view.locator(".cm-line");
    const second = await rightOf(lines.nth(1));
    await page.mouse.click(second.x, second.y);
    await expect(view.locator(".cm-activeLine")).toHaveCount(1);
    await expect(lines.nth(1)).toHaveClass(/cm-activeLine/);
    await expect(view.locator(".cm-activeLineGutter", { hasText: /^2$/ })).toBeVisible();

    // The cursor's line stands out from another line, its number too.
    const active = await pixel(page, second.x, second.y);
    const other = await rightOf(lines.nth(2));
    expect(distance(active, await pixel(page, other.x, other.y))).toBeGreaterThan(15);
    const number = view.locator(".cm-lineNumbers .cm-activeLineGutter");
    const plain = view.locator(".cm-lineNumbers .cm-gutterElement", { hasText: /^3$/ });
    const color = (l: Locator) => l.evaluate((e) => getComputedStyle(e).color);
    expect(await color(number)).not.toBe(await color(plain));

    // A selection on that line still shows through the highlight.
    await page.keyboard.press("Home");
    await page.keyboard.press("Shift+End");
    const selection = view.locator(".cm-selectionBackground").first();
    const box = await selection.boundingBox();
    if (!box) throw new Error("selection not drawn");
    const selected = await pixel(page, box.x + 1, box.y + 1);
    expect(distance(selected, active)).toBeGreaterThan(30);
    await page.screenshot({ path: `target/e2e/active-line-${theme}.png` });
  });
}
