import { expect, type Locator, type Page, test } from "@playwright/test";

// 19.14 (D19.8) against the mock transport: the endless animations rest while Hive is not
// focused, and run again when it is.

/** The play state of the first animation on `el` or its pseudo-elements. */
const playState = (el: Locator) =>
  el.evaluate((node) => node.getAnimations({ subtree: true })[0]?.playState);

const focus = (page: Page, event: "focus" | "blur") =>
  page.evaluate((type) => window.dispatchEvent(new Event(type)), event);

test("animations: the working icon spins only while Hive is focused", async ({ page }) => {
  await page.goto("/?mock=states");
  const icon = page.locator(".tree-row.agent .state-icon[data-state=working]");
  await focus(page, "focus");
  await expect.poll(() => playState(icon)).toBe("running");
  await focus(page, "blur");
  await expect.poll(() => playState(icon)).toBe("paused");
  await focus(page, "focus");
  await expect.poll(() => playState(icon)).toBe("running");
});

test("animations: the busy tab bar slides on the compositor and rests out of focus", async ({
  page,
}) => {
  await page.goto("/");
  const tab = page.getByRole("tablist").getByRole("tab", { name: "fix-login" });
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await page.keyboard.type("progress indeterminate 0");
  await page.keyboard.press("Enter");
  const bar = tab.getByRole("progressbar");
  await expect(bar).toHaveAttribute("data-state", "indeterminate");
  // A transform, never a repainted background.
  const moved = await bar.evaluate((node) => {
    const keyframes = (node.getAnimations({ subtree: true })[0] as CSSAnimation)
      .effect as KeyframeEffect;
    return keyframes.getKeyframes().flatMap((frame) => Object.keys(frame));
  });
  expect(moved).toContain("transform");
  expect(moved).not.toContain("backgroundPosition");
  await focus(page, "focus");
  await expect.poll(() => playState(bar)).toBe("running");
  await focus(page, "blur");
  await expect.poll(() => playState(bar)).toBe("paused");
  await focus(page, "focus");
  await expect.poll(() => playState(bar)).toBe("running");
});
