import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 29.12: Settings → Sharing of a remote host's own Hive, and Settings → Hosts "Join someone's
// Hive", on the mock's second host `gpu-box` (`?mock=remote`): its service checks and saves,
// alice's Hive links once joined, carol's refuses this user.

/** Opens Settings on `section`, once gpu-box is linked. */
async function settings(page: Page, section: string) {
  await page.goto("/?mock=remote");
  await expect(page.locator(".hosts-item")).toHaveText("1 host");
  await page.keyboard.press(`${mod}+Comma`);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: section, exact: true }).click();
  return dialog;
}

test("Settings → Sharing shares gpu-box's Hive with a teammate, and a refusal says why", async ({
  page,
}) => {
  const dialog = await settings(page, "Sharing");
  const form = dialog.locator("form.sharing");
  await expect(form.getByText(/your login name/)).toContainText("dev");
  const save = form.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  // On with no group: the service refuses, and the draft stays.
  await form.getByRole("checkbox", { name: "Share this Hive" }).check();
  await form.getByRole("button", { name: "Add person" }).click();
  await form.getByLabel("Login name 1").fill("bob");
  await save.click();
  await expect(form.getByRole("alert")).toHaveText("Pick the Unix group your teammates are in");
  await expect(form.getByLabel("Login name 1")).toHaveValue("bob");
  // With a group and a space: saved, nothing left to save.
  await form.getByRole("combobox", { name: "Unix group" }).click();
  await page.getByRole("option", { name: "team" }).click();
  await form.getByRole("checkbox", { name: "Default" }).check();
  await form.getByRole("combobox", { name: "Role 1" }).click();
  await page.getByRole("option", { name: "Collaborator" }).click();
  await save.click();
  await expect(save).toBeDisabled();
  await expect(form.getByRole("alert")).toHaveCount(0);
  await expect(form.getByLabel("Login name 1")).toHaveValue("bob");
  await page.screenshot({ path: "target/e2e/sharing-settings.png" });
});

test("Settings → Hosts joins a teammate's Hive; one that refuses says so and offers no restart", async ({
  page,
}) => {
  const dialog = await settings(page, "Hosts");
  const join = async (owner: string) => {
    await dialog.getByLabel("Your alias of their host").fill("gpu-box");
    await dialog.getByLabel("Their login name").fill(owner);
    await dialog.getByRole("button", { name: "Join", exact: true }).click();
  };
  await join("alice");
  await expect(dialog.locator(".host-row", { hasText: "alice@gpu-box" })).toContainText(
    "connected",
  );
  await expect(page.locator(".hosts-item")).toHaveText("2 hosts");
  await join("carol");
  const carol = dialog.locator(".host-row", { hasText: "carol@gpu-box" });
  await expect(carol).toContainText("carol's Hive on gpu-box refused you");
  await expect(carol.getByRole("button", { name: "Reconnect carol@gpu-box" })).toBeVisible();
  await expect(carol.getByRole("button", { name: /Restart/ })).toHaveCount(0);
  await expect(page.locator(".hosts-item")).toHaveText("1 of 3 hosts down");
  await page.screenshot({ path: "target/e2e/hosts-join.png" });
  // Removed: only that one goes.
  await dialog.getByRole("button", { name: "Remove carol@gpu-box" }).click();
  await expect(page.locator(".hosts-item")).toHaveText("2 hosts");
  await expect(dialog.getByRole("button", { name: "Remove gpu-box" })).toBeVisible();
});
