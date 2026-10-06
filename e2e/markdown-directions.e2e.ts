import { readFileSync } from "node:fs";
import { expect, type Page, test } from "@playwright/test";

// 14.6 PROTOTYPE: screenshots of the document directions for the human (not a gate).
const OUT = process.env.SHOTS ?? "target/e2e/14.6";
const SAMPLER = `## Sampler (14.6): what CLAUDE.md and hive.md do not have

- [x] Research how good previews render Markdown
- [ ] Show the human 2–3 directions
  - nested item with a [link to GitHub](https://github.com/lucasreali/hive) and \`inline code\`
  - a refused link: [javascript](javascript:alert(1)) stays text

> **Note:** Hive only *observes* agents; it never starts, controls or talks to them.
> A second line of the same quote.

![The prototype's screen 1g](docs/prototype/1g.png)

\`\`\`ts
// The editor's parsers colour a fenced block.
export function safeUrl(url: string): string {
  const trimmed = url.trim();
  return /^https?:/i.test(trimmed) ? trimmed : "";
}
\`\`\`

\`\`\`rust
/// A frame: \`[type][channel][length][payload]\`.
pub fn encode(kind: u8, channel: u32, payload: &[u8]) -> Result<Vec<u8>, Error> {
    let len = u32::try_from(payload.len()).map_err(|_| Error::TooLarge)?;
    Ok([&[kind][..], &channel.to_be_bytes(), &len.to_be_bytes(), payload].concat())
}
\`\`\`

---

`;
const SAMPLE =
  readFileSync("CLAUDE.md", "utf8") +
  "\n" +
  SAMPLER +
  readFileSync("docs/hive.md", "utf8").split("\n").slice(0, 245).join("\n");

const LOOKS = ["today", "a", "b", "c"] as const;

async function look(page: Page, dir: string, theme: "light" | "dark") {
  await page.evaluate(
    ([dir, theme]) => {
      if (theme === "light") document.documentElement.dataset.theme = "one-light";
      else delete document.documentElement.dataset.theme;
      for (const el of document.querySelectorAll<HTMLElement>(".markdown-view .markdown")) {
        el.classList.toggle("markdown-doc", dir !== "today");
        el.dataset.dir = dir;
      }
    },
    [dir, theme],
  );
}

test("14.6 directions: the same rich file in light and dark", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText(SAMPLE);
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();
  const rendered = view.locator(".markdown-view");
  await expect(rendered.locator(".tok-keyword").first()).toBeVisible();

  for (const dir of LOOKS) {
    for (const theme of ["light", "dark"] as const) {
      await look(page, dir, theme);
      const name = dir === "today" ? "today" : dir;
      await rendered.evaluate((el) => {
        el.scrollTop = 0;
      });
      await view.screenshot({ path: `${OUT}/${name}-${theme}.png` });
      for (const [heading, suffix] of [
        ["Sampler", "sampler"],
        ["Etapas de desenvolvimento", "tables"],
        ["Arquitetura", "code"],
      ] as const) {
        await rendered
          .getByRole("heading", { name: heading })
          .first()
          .evaluate((h) => {
            h.scrollIntoView({ block: "start" });
          });
        await view.screenshot({ path: `${OUT}/${name}-${theme}-${suffix}.png` });
      }
      const wide = await rendered.evaluate((root) => {
        const right = root.getBoundingClientRect().right;
        return [root, ...root.querySelectorAll(".md-table, table, p, pre")]
          .filter(
            (el) =>
              el.scrollWidth > el.clientWidth || el.getBoundingClientRect().right > right + 0.5,
          )
          .map((el) => el.tagName);
      });
      expect(wide, `${dir} ${theme}`).toEqual([]);
    }
  }
});

// 13.1 in every direction: the wide api.md beside the narrowest and widest panel.
for (const panelWidth of [280, 640]) {
  test(`14.6 directions keep 13.1 beside a ${panelWidth} px panel`, async ({ page }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    await page
      .getByRole("navigation", { name: "Projects" })
      .getByRole("button", { name: "fix-login" })
      .click();
    const panel = page.getByRole("region", { name: "Files" });
    await panel.getByRole("treeitem", { name: "docs" }).click();
    await panel.getByRole("treeitem", { name: "api.md" }).click();
    const view = page.getByRole("region", { name: "docs/api.md" });
    await view.getByRole("button", { name: "Show rendered Markdown" }).click();
    const rendered = view.locator(".markdown-view");
    await expect(rendered.getByRole("table").first()).toBeVisible();
    for (const dir of ["a", "b", "c"]) {
      await look(page, dir, "dark");
      const wide = await rendered.evaluate((root) => {
        const right = root.getBoundingClientRect().right;
        return [root, ...root.querySelectorAll(".md-table, table, p, pre")]
          .filter(
            (el) =>
              el.scrollWidth > el.clientWidth || el.getBoundingClientRect().right > right + 0.5,
          )
          .map((el) => el.tagName);
      });
      expect(wide, dir).toEqual([]);
      if (panelWidth === 640) await view.screenshot({ path: `${OUT}/${dir}-narrow-wrap.png` });
    }
  });
}
