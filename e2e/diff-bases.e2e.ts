import { expect, test } from "@playwright/test";

// 27.2: the Changes panel compares with an agent's last turn or a commit picked from the list,
// on the mock transport (its turns and commits are fixed data).
test("diff bases: since an agent's last turn, and since a commit picked", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  const summary = panel.locator(".files-summary");
  await expect(summary).toHaveText("5 files changed+24−49main");
  const files = panel.getByRole("tree", { name: "Files" });

  // A changed file open at the branch base shows its diff.
  await files.getByRole("treeitem", { name: /package\.json/ }).click();
  const view = page.getByRole("region", { name: "package.json" });
  await expect(view.getByText("No changes in this file.")).toHaveCount(0);

  // The menu lists the agents' turns; one picked lists only what changed since it.
  const base = panel.getByRole("combobox", { name: "Compare with" });
  await base.click();
  await expect(page.getByRole("option")).toHaveText([
    "HEAD",
    "main",
    "Since fix login redirect's last turn",
    "Since write the tests's last turn (may miss its first edit)",
    "A commit…",
  ]);
  await page.getByRole("option", { name: "Since fix login redirect's last turn" }).click();
  await expect(base).toHaveText("Since fix login redirect's last turn");
  await expect(summary).toHaveText("1 file changed+21−17Since fix login redirect's last turn");
  // The open file follows: the agent did not change it since its turn.
  await expect(view.getByText("No changes in this file.")).toBeVisible();

  // "A commit…": a searchable list, the branch's commits first; Enter picks.
  await base.click();
  await page.getByRole("option", { name: "A commit…" }).click();
  const picker = page.getByRole("dialog", { name: "Compare with a commit" });
  await expect(picker.locator(".picker-heading")).toHaveText(["This branch", "Older commits"]);
  await expect(picker.locator(".commit-short")).toHaveText([
    "a1b2c3d",
    "b2c3d4e",
    "c3d4e5f",
    "d4e5f60",
  ]);
  await picker.getByRole("textbox", { name: "Filter commits" }).fill("depend");
  await expect(picker.locator(".commit-short")).toHaveText(["c3d4e5f"]);
  await page.keyboard.press("Enter");
  await expect(picker).toBeHidden();
  await expect(base).toHaveText("c3d4e5f");
  await expect(summary).toHaveText("5 files changed+24−49c3d4e5f");
  await expect(view.getByText("No changes in this file.")).toHaveCount(0);
  await page.screenshot({ path: "target/e2e/diff-bases.png" });

  // Esc closes the picker without a pick.
  await base.click();
  await page.getByRole("option", { name: "A commit…" }).click();
  await expect(picker).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(picker).toBeHidden();
  await expect(base).toHaveText("c3d4e5f");
});

// 28.17: at the right panel's narrowest the base menu, at the panel's right edge, opens inside
// the window, every option whole.
test("diff bases: the menu stays inside the window with the panel at its narrowest", async ({
  page,
}) => {
  await page.addInitScript(() =>
    localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: 1 })),
  );
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "refactor-auth" })
    .click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  await panel.getByRole("combobox", { name: "Compare with" }).click();
  await expect(page.getByRole("option")).toHaveCount(5);
  const outside = await page.getByRole("listbox").evaluate((list) => {
    const out: string[] = [];
    for (const e of [list, ...list.querySelectorAll('[role="option"], .select-value')]) {
      const r = e.getBoundingClientRect();
      if (r.left < 0 || r.top < 0 || r.right > innerWidth || r.bottom > innerHeight) {
        out.push(e.textContent ?? "");
      }
      // A label cut by its own box is not whole either.
      if (e.scrollWidth > e.clientWidth + 1) out.push(`cut: ${e.textContent}`);
    }
    return out;
  });
  expect(outside).toEqual([]);
  await page.screenshot({ path: "target/e2e/diff-bases-narrow.png" });
});
