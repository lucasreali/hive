import { expect, type Locator, type Page, test } from "@playwright/test";

// 8.19: a placeholder is never selected. The selection API does not report a placeholder (it lives
// in the input's shadow tree), so the check is what shows: the field looks the same before and
// after selecting. Its first pixels are left out: an empty selection edge may show there.
const look = async (page: Page, field: Locator) => {
  const box = await field.boundingBox();
  if (!box) throw new Error("field not visible");
  const clip = { x: box.x + 6, y: box.y, width: box.width - 6, height: box.height };
  return page.screenshot({ clip });
};

const blur = (page: Page) =>
  page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());

const drag = async (page: Page, from: [number, number], to: [number, number]) => {
  await page.mouse.move(...from);
  await page.mouse.down();
  await page.mouse.move(...to, { steps: 10 });
  await page.mouse.up();
};

test("placeholders are never selected; typed text still is", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  const search = page.getByPlaceholder("Find files");
  await expect(search).toBeVisible();
  const box = await search.boundingBox();
  if (!box) throw new Error("search not visible");

  await search.focus();
  const focused = await look(page, search);
  await blur(page);
  const idle = await look(page, search);

  // Ctrl+A with the focus outside any field selects the page, never a placeholder.
  await page.keyboard.press("Control+a");
  expect(await page.evaluate(() => getSelection()?.type)).toBe("Range");
  expect(await look(page, search)).toEqual(idle);

  // Nor does a drag or a double-click inside it.
  await page.evaluate(() => getSelection()?.removeAllRanges());
  await drag(page, [box.x + 8, box.y + box.height / 2], [box.x + 150, box.y + box.height / 2]);
  expect(await look(page, search)).toEqual(focused);
  await page.mouse.dblclick(box.x + 40, box.y + box.height / 2);
  expect(await look(page, search)).toEqual(focused);

  // Typed text still selects, by drag and by Ctrl+A.
  await search.fill("hello world");
  const selected = () =>
    search.evaluate((el: HTMLInputElement) =>
      el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0),
    );
  await drag(
    page,
    [box.x + 4, box.y + box.height / 2],
    [box.x + box.width - 4, box.y + box.height / 2],
  );
  expect(await selected()).toBe("hello world");
  await search.press("End");
  expect(await selected()).toBe("");
  await search.press("Control+a");
  expect(await selected()).toBe("hello world");
});
