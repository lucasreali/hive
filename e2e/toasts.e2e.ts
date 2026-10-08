import { expect, test } from "@playwright/test";

test("messages: a failure is a toast bottom-right above the status bar that stays; a confirmation fades (10.3)", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.clock.install();
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const toasts = page.getByRole("status", { name: "Messages" });
  const statusbar = page.locator(".statusbar");

  // A terminal in the project, so removing it is refused.
  await tree.getByRole("button", { name: "fix-login", exact: true }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(page.getByRole("tab", { name: "fix-login" })).toBeVisible();
  await tree.getByRole("button", { name: "shop", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove project…" }).click();
  await page
    .getByRole("dialog", { name: "Remove project?" })
    .getByRole("button", { name: "Remove" })
    .click();

  const refused = toasts.locator(".toast", { hasText: "close its terminals first" });
  await expect(refused).toBeVisible();
  // Bottom-right, just above the status bar.
  const [toast, bar, view] = [
    await refused.boundingBox(),
    await statusbar.boundingBox(),
    page.viewportSize(),
  ];
  if (!toast || !bar || !view) throw new Error("no layout");
  expect(view.width - (toast.x + toast.width)).toBeLessThanOrEqual(16);
  expect(toast.y + toast.height).toBeLessThanOrEqual(bar.y);
  expect(bar.y - (toast.y + toast.height)).toBeLessThanOrEqual(16);
  // The status bar holds no message: the place, the connection, the usage ring's (hidden)
  // tooltip and the version only.
  await expect(statusbar).toHaveText(
    /^WSL(: \S+)?connectedSession 42% · resets \d\d:\d\dWeek 41% · resets \w{3} \d\d:\d\dv\d+\.\d+\.\d+$/,
  );

  // A confirmation fades after about 4 s; the error stays until dismissed.
  await tree.getByRole("button", { name: "fix-login", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Copy path" }).click();
  const copied = toasts.locator(".toast", { hasText: "Copied" });
  await expect(copied).toBeVisible();
  await page.clock.fastForward(10_000);
  await expect(copied).toHaveCount(0);
  await expect(refused).toBeVisible();
  await refused.getByRole("button", { name: "Dismiss" }).click();
  await expect(toasts.locator(".toast")).toHaveCount(0);
});
