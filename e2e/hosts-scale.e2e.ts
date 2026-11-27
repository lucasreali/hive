import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 27.8: many hosts (`?mock=remote&hosts=<n>`: gpu-box, then fake hosts `build-02` and on, a
// few of them down, of another version or refused). The space switcher is grouped by host and
// the status bar keeps one host item whatever their number. The screenshots in target/e2e, in
// both themes with 1, 3 and 50 hosts, are for the human's look.

/** Opens the page with `count` hosts in `theme`, once every link is in. */
async function open(page: Page, count: number, theme = "one-dark") {
  await page.goto(`/?mock=remote&hosts=${count}`);
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, theme);
  return tree;
}

/** What the status bar's item says once every fake host's link is in. */
const SUMMARY: Record<number, string> = {
  1: "1 host",
  3: "1 of 3 hosts down",
  50: "15 of 50 hosts down",
};

for (const theme of ["one-dark", "one-light"]) {
  for (const count of [1, 3, 50]) {
    test(`the switcher, the host item and the new space with ${count} hosts (${theme})`, async ({
      page,
    }) => {
      const tree = await open(page, count, theme);
      const item = page.locator(".hosts-item");
      await expect(item).toHaveText(SUMMARY[count]);
      const shot = (what: string) =>
        page.screenshot({ path: `target/e2e/hosts-scale-${count}-${what}-${theme}.png` });

      const space = tree.getByRole("combobox", { name: "Space" });
      await space.click();
      await expect(page.getByRole("listbox", { name: "Space" })).toBeVisible();
      await shot("switcher");
      await page.keyboard.press("Escape");

      await item.click();
      await expect(page.getByRole("dialog", { name: "Hosts" })).toBeVisible();
      await shot("popover");
      await page.keyboard.press("Escape");

      await space.click();
      await page.getByRole("option", { name: "New space…" }).click();
      const dialog = page.getByRole("dialog", { name: "New space" });
      await expect(dialog.getByRole("listbox", { name: "Where" })).toBeVisible();
      await shot("new-space");
      await dialog.getByRole("button", { name: "Cancel" }).click();

      await page.keyboard.press(`${mod}+Comma`);
      const settings = page.getByRole("dialog", { name: "Settings" });
      await settings.getByRole("button", { name: "Hosts" }).click();
      await expect(settings.getByRole("button", { name: "Remove gpu-box" })).toBeVisible();
      await shot("settings");
    });
  }
}

test("with 50 hosts the switcher and the status bar fit, and the keyboard goes through", async ({
  page,
}) => {
  const tree = await open(page, 50);
  const view = page.viewportSize() ?? { width: 0, height: 0 };
  const item = page.locator(".hosts-item");
  await expect(item).toHaveText("15 of 50 hosts down");

  // The status bar stays one line, nothing spilling out.
  const bar = page.locator(".statusbar");
  expect((await bar.boundingBox())?.height).toBeLessThanOrEqual(25);
  expect(await bar.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);

  // The switcher opens from the keyboard, into its filter; every other host collapsed.
  const space = tree.getByRole("combobox", { name: "Space" });
  await space.focus();
  await page.keyboard.press("ArrowDown");
  const filter = page.getByRole("combobox", { name: "Filter spaces" });
  await expect(filter).toBeFocused();
  const list = page.locator(".space-list");
  const box = await list.boundingBox();
  expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(view.height);
  await expect(page.getByRole("option", { name: /, collapsed$/ })).toHaveCount(50);
  // ↓ reaches a host's header, → opens it.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("option", { name: "gpu-box, 1 space, open" })).toBeVisible();
  // A host's name finds its spaces; Enter shows the first.
  await page.keyboard.type("build-07");
  await expect(page.getByRole("option", { name: "Default · build-07" })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(space).toHaveText("Default · build-07");
  await expect(space).toBeFocused();
  // Its section comes first now.
  await page.keyboard.press("ArrowDown");
  await expect(
    page.getByRole("listbox", { name: "Space" }).getByRole("option").first(),
  ).toHaveAccessibleName("Default · build-07");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox", { name: "Space" })).toHaveCount(0);

  // The host item opens its popover from the keyboard, the hosts with a problem first.
  await item.focus();
  await page.keyboard.press("Enter");
  const popover = page.getByRole("dialog", { name: "Hosts" });
  const pbox = await popover.boundingBox();
  expect(pbox?.y ?? -1).toBeGreaterThanOrEqual(0);
  const hostFilter = popover.getByRole("searchbox", { name: "Filter hosts" });
  await expect(hostFilter).toBeFocused();
  await expect(popover.locator("li").first()).toHaveAttribute("data-status", "reconnecting");
  await page.keyboard.type("build-03");
  await expect(popover.locator("li")).toHaveCount(1);
  // ↓ goes from the filter to its Reconnect, past the field's clear button.
  await page.keyboard.press("ArrowDown");
  await expect(popover.getByRole("button", { name: "Reconnect build-03" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(item).toHaveText("14 of 50 hosts down");
  await page.keyboard.press("Escape");
  await expect(popover).toHaveCount(0);
  await expect(item).toBeFocused();
});

test("Settings → Hosts restarts a host of another version and reconnects a down one (27.10)", async ({
  page,
}) => {
  await open(page, 10);
  const item = page.locator(".hosts-item");
  await expect(item).toHaveText("4 of 10 hosts down");
  await page.keyboard.press(`${mod}+Comma`);
  const settings = page.getByRole("dialog", { name: "Settings" });
  await settings.getByRole("button", { name: "Hosts" }).click();
  const hosts = settings.locator(".hosts");
  // build-05 runs another version: its row says what to do and offers the restart.
  await expect(hosts).toContainText(
    "build-05disconnected: version 0.29.0, this app 0.30.0: restart the remote service to update it",
  );
  const restarts = settings.getByRole("button", { name: /^Restart the remote service on / });
  await expect(restarts).toHaveCount(1);
  const restart = settings.getByRole("button", { name: "Restart the remote service on build-05" });
  await expect(restart).toHaveAttribute("title", "Ends its 2 terminals");
  await restart.click();
  await expect(restart).toHaveCount(0);
  await expect(hosts).toContainText("build-05connected");
  // build-03 is reconnecting, build-08 refused: each has Reconnect; a connected host none.
  await expect(settings.getByRole("button", { name: "Reconnect build-02" })).toHaveCount(0);
  await expect(settings.getByRole("button", { name: "Reconnect build-08" })).toBeVisible();
  const reconnect = settings.getByRole("button", { name: "Reconnect build-03" });
  await reconnect.click();
  await expect(reconnect).toHaveCount(0);
  await expect(item).toHaveText("2 of 10 hosts down");
});
