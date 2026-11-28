import { expect, type Locator, type Page, test } from "@playwright/test";
import { mod } from "./host";

// 29.6: inside Settings, a search field (the shared `SearchField`) has the height, padding and
// font size of Settings' other text inputs; the compact ones of the right panel, the Sessions
// and the space switcher keep their size. With 50 hosts, Settings → Hosts has its filter too.

/** The box a text field draws: the `SearchField`'s label, or the input itself. */
const box = (field: Locator) =>
  field.evaluate((e) => {
    const drawn = e.closest(".files-search-field") ?? e;
    const style = getComputedStyle(drawn);
    return {
      height: drawn.getBoundingClientRect().height,
      padding: style.paddingLeft,
      fontSize: getComputedStyle(e).fontSize,
    };
  });

/** The inner input of a search field draws nothing of its own: no border, ring or padding. */
const bare = (field: Locator) =>
  field.evaluate((e) => {
    const style = getComputedStyle(e);
    const label = (e.closest(".files-search-field") as HTMLElement).getBoundingClientRect();
    return {
      inside: e.getBoundingClientRect().height <= label.height,
      padding: style.paddingLeft,
      ring: style.boxShadow,
    };
  });

async function open(page: Page, theme: string) {
  await page.goto("/?mock=remote&hosts=50");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, theme);
}

for (const theme of ["one-dark", "one-light"]) {
  test(`Settings' search fields are as tall as its text inputs, the others stay compact (${theme})`, async ({
    page,
  }) => {
    await open(page, theme);
    // The compact ones, measured before Settings opens.
    const compact = [
      page.getByRole("searchbox", { name: "Find files" }),
      page.getByRole("searchbox", { name: "Search sessions" }),
    ];
    await expect(compact[0] as Locator).toBeVisible();
    expect((await box(compact[0] as Locator)).height).toBe(28);
    await page
      .getByRole("complementary", { name: "Side panel" })
      .getByRole("tab", { name: "Sessions" })
      .click();
    expect((await box(compact[1] as Locator)).height).toBe(28);
    await page
      .getByRole("navigation", { name: "Projects" })
      .getByRole("combobox", { name: "Space" })
      .click();
    const spaces = page.getByRole("combobox", { name: "Filter spaces" });
    await expect(spaces).toBeVisible();
    // Squeezed below 28px by the long list with 50 hosts, as before 29.6: not this task's.
    expect((await box(spaces)).height).toBeLessThanOrEqual(28);
    await page.keyboard.press("Escape");

    await page.keyboard.press(`${mod}+Comma`);
    const settings = page.getByRole("dialog", { name: "Settings" });
    await settings.getByRole("button", { name: "Worktrees", exact: true }).click();
    const input = await box(settings.getByLabel("Default base branch"));
    expect(input.height).toBe(30);

    const nav = settings.getByRole("searchbox", { name: "Search settings" });
    expect(await box(nav)).toEqual(input);

    await settings.getByRole("button", { name: "Keyboard", exact: true }).click();
    const shortcuts = settings.getByRole("searchbox", { name: "Search shortcuts" });
    expect(await box(shortcuts)).toEqual(input);
    await page.screenshot({ path: `target/e2e/settings-search-keyboard-${theme}.png` });

    await settings.getByRole("button", { name: "Hosts", exact: true }).click();
    const hosts = settings.getByRole("searchbox", { name: "Filter hosts" });
    expect(await box(hosts)).toEqual(input);
    await hosts.focus();
    expect(await bare(hosts)).toEqual({ inside: true, padding: "0px", ring: "none" });
    await page.screenshot({ path: `target/e2e/settings-search-hosts-${theme}.png` });
  });
}
