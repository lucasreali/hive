import { expect, type Page, test } from "@playwright/test";

/** What terminal `id` shows, read from the page's own terminal manager. */
function screenText(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i).translateToString(true));
    return lines.join("\n");
  }, id);
}

test("project scripts: a run script set in the settings runs from the worktree's menu", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login", exact: true }).click();
  await page.keyboard.press("Control+Comma");
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Projects" }).click();
  await expect(dialog.getByRole("combobox", { name: "Project" })).toHaveText("shop");
  await dialog.getByLabel("New run script name").fill("greet");
  await dialog.getByLabel("New run script command").fill("echo hello-from-greet");
  await dialog.getByRole("button", { name: "Add" }).click();
  await expect(dialog.getByLabel("Command of greet")).toHaveValue("echo hello-from-greet");
  await page.screenshot({ path: "target/e2e/project-scripts.png" });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  await tree.getByRole("button", { name: "fix-login", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Run: greet" }).click();
  await expect(page.getByRole("tab", { name: /fix-login$/ })).toBeVisible();
  // The mock terminal echoes what was typed, and repeats the line on Enter.
  await expect
    .poll(() => screenText(page, 1))
    .toMatch(/echo hello-from-greet[\s\S]*echo hello-from-greet/);
});
