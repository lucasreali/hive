import { expect, type Page, test } from "@playwright/test";

/** The text of terminal 1's buffer, read from the page's own terminal manager. */
function screen(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(1).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n");
  });
}

// 23.4: the right panel's Scripts tab on the fake service: shop's repository scripts by source,
// the worktree's ports, a run in a new terminal, shown as running until stopped; the empty state
// opens the project's scripts in Settings.
test("scripts: listed by source, run in a terminal, stopped; none points to Settings", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tab", { name: "Scripts" }).click();
  const view = panel.getByRole("region", { name: "Scripts" });
  await expect(view.locator(".scripts-group h3")).toHaveText([
    "package.json",
    "Taskfile",
    "Makefile",
  ]);
  await expect(view.locator(".scripts-ports")).toContainText("Ports 20000–20009");
  await expect(view.locator(".scripts-problem")).toHaveText(
    "Makefile: 1 script left out (a name Hive cannot type as one word)",
  );

  await view.getByTitle("Run in a new terminal: task db:up").click();
  // Typed into a new terminal (the fake shell echoes it).
  await expect.poll(() => screen(page)).toContain("task db:up");
  const running = view.getByRole("region", { name: "Running" });
  await expect(running.locator(".script")).toHaveText(["db:uptask db:up"]);
  await running.getByTitle("Stop db:up (sends Ctrl+C)").click();
  await expect(running).toHaveCount(0);

  // A project without scripts: Settings opens on its scripts.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  await view.getByRole("button", { name: "Open Settings" }).click();
  const settings = page.getByRole("dialog");
  await expect(settings.getByRole("combobox", { name: "Project" })).toContainText("api");
});
