import { expect, type Locator, type Page, test } from "@playwright/test";

/** Shows the given sessions (copies of main's "Checkout totals") as the service would, in this order. */
const show = (page: Page, rows: [string, string | null][]) =>
  page.evaluate(async (rows) => {
    const [{ transport }, { apply }, { useHive }] = await Promise.all(
      ["/src/transport/index.ts", "/src/reduce.ts", "/src/store.ts"].map(
        (url) => import(/* @vite-ignore */ url),
      ),
    );
    // The 5 s refresh would bring the mock's own sessions back.
    transport.listSessions = async () => {};
    // Kept from the first call: later ones replace the mock's sessions.
    const kept = window as unknown as { base?: object };
    kept.base ??= useHive
      .getState()
      .sessions.find((x: { title: string }) => x.title === "Checkout totals");
    const { base } = kept;
    apply({
      type: "sessions",
      sessions: rows.map(([title, last]) => ({ ...base, id: title, title, last_text: last })),
      error: null,
      truncated: false,
    });
  }, rows);

/** Each row starts at or below the end of the one before it. */
async function noOverlap(rows: Locator, titles: string[]) {
  await expect(rows.locator(".session-title .label")).toHaveText(titles);
  const boxes = await rows.evaluateAll((li) => li.map((e) => e.getBoundingClientRect().toJSON()));
  for (let i = 1; i < boxes.length; i++) {
    expect(boxes[i].top).toBeGreaterThanOrEqual(boxes[i - 1].bottom - 0.5);
  }
}

// 16.2: a row's measured height follows its session when the list reorders or filters.
test("session cards never overlap after a reorder or a search", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "main", exact: true })
    .first()
    .click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel
    .getByRole("tablist", { name: "Panel" })
    .getByRole("tab", { name: "Sessions" })
    .click();
  const rows = panel.getByRole("list", { name: "Sessions" }).locator(".session");
  await expect(rows).toHaveCount(2);

  const long = "Moved the token check into its own middleware. ".repeat(6);
  const short: [string, string | null] = ["Short", null];
  const one: [string, string | null] = ["One line", "Done."];
  const two: [string, string | null] = ["Two lines", long];
  await show(page, [short, one, two]);
  await noOverlap(rows, ["Short", "One line", "Two lines"]);
  const height = (n: number) => rows.nth(n).evaluate((e) => e.getBoundingClientRect().height);
  // Three different heights, so a size left at another session's index shows.
  expect(await height(0)).toBeLessThan(await height(1));
  expect(await height(1)).toBeLessThan(await height(2));

  // The most recent moves up, as the refresh does.
  await show(page, [one, two, short]);
  await noOverlap(rows, ["One line", "Two lines", "Short"]);

  // A search drops the first one; the others move up an index.
  await panel.getByRole("searchbox", { name: "Search sessions" }).fill("line");
  await show(page, [short, two, one]);
  await noOverlap(rows, ["Two lines", "One line"]);
});
