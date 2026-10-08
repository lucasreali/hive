import { expect, type Locator, type Page, test } from "@playwright/test";

/** The colour on screen at a 1×1 px point, as [r, g, b]. */
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

/** The colour `tokens` paint, each laid over the one before it, as [r, g, b]. */
const painted = (page: Page, tokens: string[]) =>
  page.evaluate((tokens) => {
    const rgba = (token: string) => {
      const probe = document.createElement("div");
      probe.style.backgroundColor = `var(${token})`;
      document.body.append(probe);
      const value = getComputedStyle(probe).backgroundColor;
      probe.remove();
      const [r, g, b, a = 1] = (value.match(/[\d.]+/g) ?? []).map(Number);
      return { rgb: [r, g, b], a };
    };
    let out = [0, 0, 0];
    for (const { rgb, a } of tokens.map(rgba)) out = out.map((v, i) => v + (rgb[i] - v) * a);
    return out;
  }, tokens);

const distance = (a: number[], b: number[]) => a.reduce((sum, v, i) => sum + Math.abs(v - b[i]), 0);

/** A point inside the selection drawn on `line`, near its top left (clear of the text). */
async function selected(view: Locator, line: Locator) {
  const row = await line.boundingBox();
  if (!row) throw new Error("line not shown");
  for (const rect of await view.locator(".cm-selectionBackground").all()) {
    const box = await rect.boundingBox();
    if (box && box.y <= row.y + row.height / 2 && box.y + box.height >= row.y + row.height / 2) {
      return { x: box.x + 2, y: row.y + 1 };
    }
  }
  throw new Error("no selection drawn on the line");
}

for (const theme of ["one-dark", "one-light"]) {
  test(`selection: the editor's selection is in ${theme}'s colour, on the cursor's line and off it, over change markers (15.1)`, async ({
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
    await expect(view.locator(".cm-content")).toBeVisible();
    // The settings have loaded by now, so the theme set here stays.
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await expect
      .poll(() =>
        page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--bg")),
      )
      .toBe(theme === "one-light" ? "#fafafa" : "#282c33");

    // The read-only diff (`createViewer`) uses the browser's own selection.
    const native = await view
      .locator(".cm-line")
      .first()
      .evaluate((e) => getComputedStyle(e, "::selection").backgroundColor);
    expect(native).toBe(
      await page.evaluate(() => {
        const probe = document.createElement("div");
        probe.style.backgroundColor = "var(--editor-selection)";
        document.body.append(probe);
        const value = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return value;
      }),
    );

    // The editor, its line 2 marked as modified against HEAD (14.4).
    await view.getByRole("button", { name: "Edit", exact: true }).click();
    const lines = view.locator(".cm-line");
    await expect(view.locator(".cm-changeGutter .cm-change-modified").first()).toBeVisible();
    await lines.first().click();
    await page.keyboard.press("Control+Home");
    for (let i = 0; i < 2; i++) await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+End");
    await expect(lines.nth(2)).toHaveClass(/cm-activeLine/);

    // Off the cursor's line: the token over the background; on it: the line's tint over that.
    const off = await selected(view, lines.nth(1));
    expect(
      distance(
        await pixel(page, off.x, off.y),
        await painted(page, ["--bg", "--editor-selection"]),
      ),
    ).toBeLessThan(8);
    const on = await selected(view, lines.nth(2));
    const onLine = await pixel(page, on.x, on.y);
    expect(
      distance(onLine, await painted(page, ["--bg", "--editor-selection", "--active-line"])),
    ).toBeLessThan(8);
    // Both stand out from the unselected background.
    const plain = await painted(page, ["--bg"]);
    expect(distance(await pixel(page, off.x, off.y), plain)).toBeGreaterThan(30);
    expect(distance(onLine, plain)).toBeGreaterThan(30);
    await page.screenshot({ path: `target/e2e/selection-${theme}.png` });
  });
}
