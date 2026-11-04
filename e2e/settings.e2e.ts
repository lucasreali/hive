import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

/** Terminal 1's font size and columns, read from the page's own terminal manager. */
function terminal(page: Page): Promise<{ fontSize: number; cols: number }> {
  return page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const term = terminal(1);
    return { fontSize: term.options.fontSize, cols: term.cols };
  });
}

const cssVar = (page: Page, name: string) =>
  page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name), name);

test("settings: Ctrl+, opens them; font size and theme apply live", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(async () => (await terminal(page)).fontSize).toBe(13);
  const before = await terminal(page);

  // Taken even with the focus in the terminal.
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Font size", { exact: true }).fill("19");
  // The stepper's own button, not the WebView's spin button.
  await dialog.getByRole("button", { name: "Increase Font size" }).click();
  await expect(dialog.getByLabel("Font size", { exact: true })).toHaveValue("20");
  await expect.poll(async () => (await terminal(page)).fontSize).toBe(20);
  // Bigger cells: the terminal refits to fewer columns.
  expect((await terminal(page)).cols).toBeLessThan(before.cols);

  expect(await cssVar(page, "--bg")).toBe("#282c33");
  await dialog.getByRole("button", { name: "Appearance" }).click();
  await dialog.getByRole("combobox", { name: "Theme" }).click();
  await page.getByRole("option", { name: "One Light" }).click();
  await expect.poll(() => cssVar(page, "--bg")).toBe("#fafafa");

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("settings: a new account's folder is browsed from home, hidden ones included (22.2)", async ({
  page,
}) => {
  await page.goto("/");
  // Once the app is up: the shortcut is the app's.
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Accounts" }).click();
  // Hive makes the folder unless an existing one is chosen (22.3).
  await dialog.getByRole("button", { name: "New account" }).click();
  await dialog.getByLabel("Use an existing folder").check();
  const folder = dialog.getByLabel("New account folder");
  await expect(folder).toHaveValue("/home/user/");
  const folders = dialog.getByRole("list", { name: "Folders" });
  await folders.getByRole("button", { name: ".claude-work" }).click();
  await expect(folder).toHaveValue("/home/user/.claude-work/");
  await expect(folder).toBeFocused();
  const claude = dialog.getByText("A Claude config folder");
  await expect(claude).toBeVisible();
  await folders.getByTitle("Up a level").click();
  await expect(folder).toHaveValue("/home/user/");
  await expect(claude).toBeHidden();
  // A typed path still works.
  await folder.fill("/home/user/.claude-work");
  await dialog.getByLabel("New account name").fill("Work");
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  await expect(dialog.getByTitle("/home/user/.claude-work")).toHaveText("/home/user/.claude-work");
  // By name only: Hive makes its folder, and offers to log in to it.
  await dialog.getByRole("button", { name: "New account" }).click();
  await dialog.getByLabel("Use an existing folder").uncheck();
  await expect(folder).toBeHidden();
  await dialog.getByLabel("New account name").fill("Personal");
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  const made = "/home/mock/.hive/accounts/personal";
  await expect(dialog.getByTitle(made)).toHaveText(made);
  await expect(dialog.getByRole("status")).toHaveText("Personal is made: log in to it.");
});

test("settings: Accounts fits the dialog narrow and wide, no button clipped (22.6)", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Accounts" }).click();
  // Two accounts besides the default one, a long folder among them; the mock says neither
  // logged in, so both rows carry the badge.
  const long = "/home/user/clients/ai3-visus-consulting/claude-accounts/.claude-work-long";
  await dialog.getByRole("button", { name: "New account" }).click();
  await dialog.getByLabel("New account name").fill("Work");
  await dialog.getByLabel("Use an existing folder").check();
  await dialog.getByLabel("New account folder").fill(long);
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  await dialog.getByRole("button", { name: "New account" }).click();
  await dialog.getByLabel("Use an existing folder").uncheck();
  await dialog.getByLabel("New account name").fill("Ai3 - Visus");
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  await expect(dialog.getByText("Not logged in")).toHaveCount(2);
  const group = dialog.getByRole("region", { name: "Claude accounts" });
  // The app's smallest window, a narrower one (the dialog shrinks below 792 px) and a wide one.
  for (const [width, height] of [
    [800, 500],
    [640, 600],
    [1440, 900],
  ]) {
    await page.setViewportSize({ width, height });
    const box = await group.boundingBox();
    const buttons = group.getByRole("button");
    expect(await buttons.count()).toBe(6);
    for (const button of await buttons.all()) {
      const own = await button.boundingBox();
      expect(own && box && own.x >= box.x && own.x + own.width <= box.x + box.width + 0.5).toBe(
        true,
      );
      // Its label fits it.
      expect(await button.evaluate((b) => b.scrollWidth <= b.clientWidth)).toBe(true);
    }
    // Each row's "Log in…" sits in the same column.
    const xs = await group
      .getByRole("button", { name: "Log in…" })
      .evaluateAll((all) => all.map((b) => Math.round(b.getBoundingClientRect().x)));
    expect(new Set(xs).size).toBe(1);
    // The long folder keeps its last segment and is one path: no gap before it.
    const folder = group.getByTitle(long);
    await expect(folder).toContainText("/.claude-work-long");
    // Its start is cut to whole characters, so the ellipsis meets the last segment.
    const chars = await folder.evaluate((f) => {
      const [head, tail] = [...f.children].map((c) => c.getBoundingClientRect().width);
      return (head as number) / ((tail as number) / "/.claude-work-long".length);
    });
    expect(Math.abs(chars - Math.round(chars))).toBeLessThan(0.05);
  }
});
