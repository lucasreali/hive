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
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await page.getByRole("button", { name: "Start chat" }).click();
  const input = page
    .getByRole("region", { name: "Chat" })
    .getByRole("textbox", { name: "Message" });
  await expect(input).toHaveAttribute("placeholder", "Message Claude — / for commands");
  const search = page.getByPlaceholder("Find files");
  await expect(search).toBeVisible();
  const box = await input.boundingBox();
  if (!box) throw new Error("composer not visible");

  await input.focus();
  const composerFocused = await look(page, input);
  await blur(page);
  const composerIdle = await look(page, input);
  const searchIdle = await look(page, search);

  // Ctrl+A with the focus outside any field selects the page, never a placeholder.
  await page.keyboard.press("Control+a");
  expect(await page.evaluate(() => getSelection()?.type)).toBe("Range");
  expect(await look(page, input)).toEqual(composerIdle);
  expect(await look(page, search)).toEqual(searchIdle);

  // A drag from the transcript down across the empty composer selects nothing in it.
  await page.evaluate(() => getSelection()?.removeAllRanges());
  await drag(page, [box.x + 20, box.y - 80], [box.x + box.width - 20, box.y + box.height + 10]);
  await blur(page);
  expect(await look(page, input)).toEqual(composerIdle);
  // Nor does a drag or a double-click inside it.
  await drag(page, [box.x + 8, box.y + 10], [box.x + 250, box.y + 10]);
  expect(await look(page, input)).toEqual(composerFocused);
  await page.mouse.dblclick(box.x + 40, box.y + 10);
  expect(await look(page, input)).toEqual(composerFocused);

  // Typed text still selects, by drag and by Ctrl+A.
  await input.fill("hello world");
  const selected = () =>
    input.evaluate((el: HTMLTextAreaElement) => el.value.slice(el.selectionStart, el.selectionEnd));
  await drag(page, [box.x + 4, box.y + 10], [box.x + 250, box.y + 10]);
  expect(await selected()).toBe("hello world");
  await input.press("End");
  expect(await selected()).toBe("");
  await input.press("Control+a");
  expect(await selected()).toBe("hello world");
});
