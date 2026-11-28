import { expect, type Page, test } from "@playwright/test";
import { mac, mod } from "./host";

// 29.2 (docs/spike/pinned-tabs.md): a pinned tab has a pin in its ×'s place, and every way the
// user closes a tab leaves it open (its menu's Close is disabled; ⌘W and the palette's Close show
// a toast); it keeps its place, and its pin comes back after a reload.

/** Runs the palette's command `name`. */
async function palette(page: Page, name: string) {
  await page.keyboard.press(`${mod}+Shift+P`);
  const dialog = page.getByRole("dialog", { name: "Command palette" });
  await dialog.getByRole("textbox", { name: /Search commands/ }).fill(name);
  await dialog.getByRole("button", { name: new RegExp(`^${name}`) }).click();
  await expect(dialog).toBeHidden();
}

for (const theme of ["one-dark", "one-light"]) {
  test(`a pinned chat and file stay open every way they are closed, after a reload too, in ${theme} (29.2)`, async ({
    page,
  }) => {
    // Once the settings have loaded (the tree is listed), the theme set here stays.
    const setTheme = async () => {
      await page.evaluate((t) => {
        document.documentElement.dataset.theme = t;
      }, theme);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    };
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    const bar = page.getByRole("tablist", { name: "Open terminals and files" });
    const tabs = bar.getByRole("tab");
    const toasts = page.getByRole("status", { name: "Messages" });
    const menu = page.getByRole("menu");
    /** Opens `name` from the Files view in a tab of its own (a double click keeps it, 11.1). */
    const open = async (name: string) => {
      await page
        .getByRole("region", { name: "Files" })
        .getByRole("treeitem", { name, exact: true })
        .dblclick();
      await expect(page.getByRole("region", { name }).locator(".cm-content")).not.toBeEmpty();
    };
    /** A terminal in fix-login running `claude` (the mock names its session by the channel). */
    const chat = async () => {
      await page.getByTitle("New terminal or file").click();
      await page.getByRole("menuitem", { name: "Terminal" }).click();
      await page.locator(".xterm-helper-textarea").first().focus();
      await page.keyboard.type("claude");
      await page.keyboard.press("Enter");
      await expect(bar.locator(".tab .state-icon")).toHaveCount(1);
    };

    await tree.getByRole("button", { name: "fix-login" }).click();
    await setTheme();
    await chat();
    await open("README.md");
    await open("package.json");
    // The chat dragged between the files (8.21), so the order holds its session for a reload.
    const item = (name: string) => bar.locator(".tab", { hasText: name });
    await item("fix-login").dragTo(item("package.json"), { targetPosition: { x: 2, y: 10 } });
    const chatTab = item("fix-login").getByRole("tab");
    const order = ["README.md", /fix-login$/, "package.json"];
    await expect(tabs).toHaveText(order);

    // Pin the chat from its menu: a pin in its ×'s place, which does nothing when clicked.
    await chatTab.click({ button: "right" });
    await menu.getByRole("menuitem", { name: "Pin tab" }).click();
    const pinned = bar.locator(".tab[data-pinned]");
    await expect(pinned).toHaveCount(1);
    await expect(chatTab).toHaveAttribute("aria-description", "Pinned");
    await expect(pinned.getByRole("button")).toHaveCount(0);
    await pinned.locator(".tab-pin").click();
    await expect(tabs).toHaveText(order);
    // The pin in the muted text colour of the theme.
    const muted = await page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--text-3").trim(),
    );
    const rgb = (hex: string) =>
      `rgb(${[1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;
    await expect(pinned.locator(".tab-pin")).toHaveCSS("color", rgb(muted));
    await page.screenshot({ path: `target/e2e/pinned-tab-${theme}.png` });

    // Its menu: Close disabled, saying why.
    await chatTab.click({ button: "right" });
    await expect(menu.getByRole("menuitem", { name: /^Close terminal/ })).toBeDisabled();
    await expect(menu.getByRole("menuitem", { name: /^Close terminal/ })).toContainText(
      "Unpin to close",
    );
    await page.keyboard.press("Escape");

    // The palette's Close, and ⌘W on macOS (no default elsewhere): it stays, with a toast.
    await chatTab.click();
    await palette(page, "Close the tab");
    await expect(toasts).toContainText("is pinned. Unpin it to close it.");
    if (mac) await page.keyboard.press("Meta+W");
    await expect(tabs).toHaveText(order);

    // The palette pins the file shown; its × is gone and it stays where it was.
    await bar.getByRole("tab", { name: "package.json" }).click();
    await palette(page, "Pin the tab");
    await expect(pinned).toHaveCount(2);
    await palette(page, "Close the tab");
    await expect(tabs).toHaveText(order);

    // A reload: the file is pinned in its place; the chat, resumed (the mock's next `claude` in
    // fix-login has the same session), is pinned again.
    await page.reload();
    await tree.getByRole("button", { name: "fix-login" }).click();
    await setTheme();
    await expect(bar.getByRole("tab", { name: "package.json" })).toBeVisible();
    await expect(tabs).toHaveText(["README.md", "package.json"]);
    await expect(pinned).toHaveCount(1);
    await expect(bar.getByRole("tab", { name: "package.json" })).toHaveAttribute(
      "aria-description",
      "Pinned",
    );
    await chat();
    await expect(pinned).toHaveCount(2);
    // Both in their places.
    await expect(tabs).toHaveText(order);
    await page.screenshot({ path: `target/e2e/pinned-tabs-reload-${theme}.png` });

    // Unpinned, it closes.
    const resumed = bar.locator(".tab", { has: page.locator(".state-icon") });
    await resumed.getByRole("tab").click({ button: "right" });
    await menu.getByRole("menuitem", { name: "Unpin tab" }).click();
    await resumed.getByRole("tab").click({ button: "right" });
    await menu.getByRole("menuitem", { name: "Close terminal" }).click();
    await expect(tabs).toHaveText(["README.md", "package.json"]);
  });
}
