import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 26.18: a remote host in the interface, with the mock's second host `gpu-box` (`?mock=remote`):
// the space switcher lists its spaces, a terminal of a host that is down shows "Reconnecting to
// gpu-box…" with its input blocked, and quitting asks whether its agents keep running. In both
// themes; the screenshots in target/e2e are for the human's look (the ⏸ checkpoint).

/** Opens the page in `theme`, once the settings (which set it) are in. */
async function open(page: Page, theme: string) {
  await page.goto("/?mock=remote");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.getByRole("button", { name: "fix-login" })).toBeVisible();
  await expect(page.locator(".hosts-item")).toHaveText("1 host");
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, theme);
  return tree;
}

/** A terminal in the selected worktree, as the tab bar's "+" opens one. */
async function newTerminal(page: Page) {
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
}

/** Types `line` and Enter into the focused terminal. */
async function type(page: Page, line: string) {
  await page.keyboard.type(line);
  await page.keyboard.press("Enter");
}

/** The text of terminal `id`'s buffer, read from the page's own terminal manager. */
function screen(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n");
  }, id);
}

/** Shows the space `name` ("Default · gpu-box") from the switcher. */
async function pick(page: Page, name: string) {
  await page.getByRole("combobox", { name: "Space" }).click();
  await page.getByRole("option", { name, exact: true }).click();
}

for (const theme of ["one-dark", "one-light"]) {
  test(`the switcher lists each connected host's spaces as "Space · host" (${theme})`, async ({
    page,
  }) => {
    const tree = await open(page, theme);
    const item = page.locator(".hosts-item");
    await expect(item).toHaveAttribute("data-status", "connected");
    await expect(item).toHaveText("1 host");
    const space = tree.getByRole("combobox", { name: "Space" });
    await expect(space).toHaveText("Default");
    await space.click();
    for (const name of ["Default", "Default · gpu-box", "New space…", "Edit space…"]) {
      await expect(page.getByRole("option", { name, exact: true })).toBeVisible();
    }
    await expect(page.locator(".space-host")).toHaveText(["This Mac", "gpu-box"]);
    await page.screenshot({ path: `target/e2e/hosts-switcher-${theme}.png` });
    await page.keyboard.press("Escape");

    // The host's spaces show its own projects, never this Mac's.
    await pick(page, "Default · gpu-box");
    await expect(space).toHaveText("Default · gpu-box");
    await expect(tree.getByRole("button", { name: "models", exact: true })).toBeVisible();
    await expect(tree.getByRole("button", { name: "shop", exact: true })).toHaveCount(0);
    await page.screenshot({ path: `target/e2e/hosts-remote-space-${theme}.png` });

    // A new space is made on the host picked in its dialog.
    await pick(page, "Default");
    await space.click();
    await page.getByRole("option", { name: "New space…" }).click();
    const dialog = page.getByRole("dialog", { name: "New space" });
    const where = dialog.getByRole("listbox", { name: "Where" });
    await expect(where.getByRole("option", { name: "This Mac" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await where.getByRole("option", { name: "gpu-box" }).click();
    await expect(where.getByRole("option", { name: "gpu-box" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await dialog.getByLabel("Name", { exact: true }).fill("Training");
    await dialog.getByLabel("Name", { exact: true }).press("Enter");
    await expect(space).toHaveText("Training · gpu-box");

    // And back to this Mac's.
    await pick(page, "Default");
    await expect(tree.getByRole("button", { name: "shop", exact: true })).toBeVisible();
    await expect(tree.getByRole("button", { name: "models", exact: true })).toHaveCount(0);
  });

  test(`a down host's terminal shows "Reconnecting to <host>…" and takes no input (${theme})`, async ({
    page,
  }) => {
    await open(page, theme);
    await pick(page, "Default · gpu-box");
    await newTerminal(page);
    await type(page, "echo up");
    const shown = () => screen(page, 1);
    await expect.poll(shown).toContain("echo up");

    await type(page, "drop-link");
    const overlay = page.getByRole("status").filter({ hasText: "Reconnecting to gpu-box…" });
    await expect(overlay.first()).toBeVisible();
    const item = page.locator(".hosts-item");
    await expect(item).toHaveText("1 host down");
    await expect(item).toHaveAttribute("data-status", "disconnected");
    // Its buffer stays; what is typed goes nowhere.
    await page.keyboard.type("lost");
    await page.waitForTimeout(300);
    expect(await shown()).toContain("echo up");
    expect(await shown()).not.toContain("lost");
    await page.screenshot({ path: `target/e2e/hosts-overlay-${theme}.png` });

    await item.click();
    const popover = page.getByRole("dialog", { name: "Hosts" });
    await expect(popover).toContainText("reconnecting since");
    await popover.getByRole("button", { name: "Reconnect gpu-box" }).click();
    await expect(item).toHaveAttribute("data-status", "connected");
    await expect(item).toHaveText("1 host");
    await expect(page.locator(".host-overlay")).toHaveCount(0);
  });

  test(`quitting asks whether a host's agents keep running, keep first (${theme})`, async ({
    page,
  }) => {
    const tree = await open(page, theme);
    await pick(page, "Default · gpu-box");
    await newTerminal(page);
    await type(page, "claude");
    await type(page, "state working");
    await expect(tree.getByRole("img", { name: "working" })).toHaveCount(1);

    // ⌘Q from this Mac's terminal (the mock's `quit-app`): no local agent, yet it asks.
    await pick(page, "Default");
    await tree.getByRole("button", { name: "main", exact: true }).first().click();
    await newTerminal(page);
    await type(page, "quit-app");
    const dialog = page.getByRole("dialog", { name: "Close Hive?" });
    const keep = dialog.getByRole("checkbox", { name: "Keep 1 agent running on gpu-box?" });
    await expect(keep).toBeChecked();
    await page.screenshot({ path: `target/e2e/hosts-quit-${theme}.png` });
    await keep.uncheck();
    await dialog.getByRole("button", { name: "Close Hive" }).click();
    await expect
      .poll(() => page.evaluate(() => "closed" in document.documentElement.dataset))
      .toBe(true);
  });
}

test("Settings → Hosts adds a host by its alias, shows its link, tests and removes it", async ({
  page,
}) => {
  await open(page, "one-dark");
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Hosts" }).click();
  await expect(dialog.getByText("connected", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("Keep remote terminals (minutes)", { exact: true })).toHaveValue(
    "1440",
  );
  // An alias the mock cannot reach stops with ssh's reason.
  await dialog.getByLabel("Host alias").fill("lab");
  await dialog.getByRole("button", { name: "Add host" }).click();
  await expect(dialog.getByText(/disconnected: ssh: Could not resolve hostname lab/)).toBeVisible();
  await expect(page.locator(".hosts-item")).toHaveText("1 of 2 hosts down");
  await page.screenshot({ path: "target/e2e/hosts-settings.png" });
  await dialog.getByRole("button", { name: "Remove lab" }).click();
  await expect(page.locator(".hosts-item")).toHaveText("1 host");
  await expect(dialog.getByRole("button", { name: "Remove gpu-box" })).toBeVisible();
});
