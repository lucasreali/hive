import { expect, test } from "@playwright/test";

// WCAG contrast ratio of two `rgb(r, g, b)` colours.
const contrast = (a: string, b: string) => {
  const luminance = (rgb: string) => {
    const [r, g, bl] = (rgb.match(/\d+/g) ?? []).slice(0, 3).map((v) => {
      const c = Number(v) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [lo, hi] = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (hi + 0.05) / (lo + 0.05);
};

// 16.1: a disabled `primary danger` button looks like a disabled `.primary`, in both themes.
for (const theme of ["one-dark", "one-light"]) {
  test(`a disabled danger button is readable in ${theme} (16.1)`, async ({ page }) => {
    await page.goto("/");
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    const tree = page.getByRole("navigation", { name: "Projects" });
    // api has no merged worktree, so there is nothing to remove.
    await tree.getByRole("button", { name: "api", exact: true }).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Remove merged worktrees…" }).click();
    const dialog = page.getByRole("dialog", { name: "Remove merged worktrees" });
    const remove = dialog.getByRole("button", { name: "Remove (0)" });
    await expect(remove).toBeDisabled();

    const colours = () =>
      remove.evaluate((el) => {
        const s = getComputedStyle(el);
        return { background: s.backgroundColor, color: s.color };
      });
    const text3 = await page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--text-3").trim(),
    );
    const rgb = (hex: string) =>
      `rgb(${[1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;

    // A disabled `.primary`'s colours: no red, readable text (on the red it was about 1:1).
    const still = await colours();
    expect(still).toEqual({ background: rgb("#3e4a5c"), color: rgb(text3) });
    expect(contrast(still.background, still.color)).toBeGreaterThan(2);

    // Hovering changes nothing.
    await remove.hover();
    expect(await colours()).toEqual(still);
  });
}
