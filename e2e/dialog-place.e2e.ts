import { expect, type Locator, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 23.9: a dialog taller than the room under its usual top is centred in the window, its body
// scrolling with the footer on screen; a short one stays where it always was.
test.use({ viewport: { width: 1440, height: 700 } });

/** The space above and below `dialog`, and its footer's buttons on screen. */
async function place(dialog: Locator) {
  return dialog.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const buttons = [
      ...el.querySelectorAll(":scope > footer button, :scope > form > footer button"),
    ];
    const shown = buttons.every((b) => {
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return r.bottom <= window.innerHeight && hit !== null && b.contains(hit);
    });
    return {
      above: box.top,
      below: window.innerHeight - box.bottom,
      buttons: buttons.length,
      shown,
    };
  });
}

/** Once `dialog` has finished gliding into place. */
const settled = (dialog: Locator) =>
  dialog.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));

/** `dialog` is centred within a few pixels, at least 24px from each edge, its footer usable. */
async function centred(dialog: Locator) {
  // Placed once its height is known, gliding into place.
  await expect
    .poll(async () => {
      const { above, below } = await place(dialog);
      return Math.abs(above - below);
    })
    .toBeLessThanOrEqual(3);
  await settled(dialog);
  const { above, below, buttons, shown } = await place(dialog);
  expect(above).toBeGreaterThanOrEqual(24);
  expect(below).toBeGreaterThanOrEqual(24);
  expect(buttons).toBeGreaterThan(0);
  expect(shown).toBe(true);
}

async function editSpace(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("combobox", { name: "Space" })
    .click();
  await page.getByRole("option", { name: "Edit space…" }).click();
  return page.getByRole("dialog", { name: "Edit space" });
}

test("Edit space at a short window is centred, its body scrolls, its footer shows", async ({
  page,
}) => {
  // Without its gh folder list (24.13), the dialog fits 700px: a shorter window.
  await page.setViewportSize({ width: 1440, height: 560 });
  const dialog = await editSpace(page);
  await expect(dialog).toBeVisible();
  await centred(dialog);
  // Taller than the window: the body scrolls between the header and the footer.
  const body = dialog.locator(".dialog-body");
  const scrolls = await body.evaluate((el) => el.scrollHeight > el.clientHeight);
  expect(scrolls).toBe(true);
  await dialog.getByRole("button", { name: /^Save/ }).click();
  await expect(dialog).toHaveCount(0);
});

test("a dialog that grows while open moves up, and stays there when it shrinks back", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove project…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove project?" });
  await expect(dialog).toBeVisible();
  // Content that comes and goes (a warning, a list), as the dialog's own would.
  const grow = (px: number) =>
    dialog.locator(".dialog-body").evaluate((body, height) => {
      body.querySelector(".test-filler")?.remove();
      if (height === 0) return;
      const filler = document.createElement("div");
      filler.className = "test-filler";
      filler.style.height = `${height}px`;
      body.append(filler);
    }, px);
  await grow(420);
  await centred(dialog);
  const grown = await place(dialog);
  expect(grown.above).toBeLessThan(120);
  await grow(0);
  await page.waitForTimeout(300);
  await settled(dialog);
  expect((await place(dialog)).above).toBeCloseTo(grown.above, 0);
});

test("Settings in a short window is centred, its footer on screen", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();
  await centred(dialog);
});

test("a short confirm dialog and the palette keep their places", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove project…" }).click();
  const confirm = page.getByRole("dialog", { name: "Remove project?" });
  await expect(confirm).toBeVisible();
  await page.waitForTimeout(300);
  const short = await place(confirm);
  expect(short.above).toBe(120);
  expect(short.shown).toBe(true);
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(confirm).toHaveCount(0);

  await page.keyboard.press(`${mod}+Shift+P`);
  const palette = page.locator(".dialog.palette");
  await expect(palette).toBeVisible();
  await page.waitForTimeout(300);
  expect((await place(palette)).above).toBe(44);
});
