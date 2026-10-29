import { expect, test } from "@playwright/test";
import { keyLabel, mod } from "./host";

test("markdown: the eye and Ctrl+Shift+V show a Markdown file rendered; Ctrl+S shows the text (11.2)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  const panel = page.getByRole("region", { name: "Files" });

  // Not a Markdown file: no eye.
  await panel.getByRole("treeitem", { name: "package.json" }).click();
  const pkg = page.getByRole("region", { name: "package.json" });
  await expect(pkg.locator(".cm-content")).toBeVisible();
  await expect(pkg.getByRole("button", { name: "Show rendered Markdown" })).toHaveCount(0);

  await panel.getByRole("treeitem", { name: "README.md" }).click();
  const view = page.getByRole("region", { name: "README.md" });
  const eye = view.getByRole("button", { name: "Show rendered Markdown" });
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(eye).toHaveAttribute("title", keyLabel("Show rendered (Ctrl+Shift+V)"));
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+Home`);
  await page.keyboard.type("# Hello\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n");

  // The unsaved text, rendered.
  await eye.click();
  await expect(eye).toHaveAttribute("aria-pressed", "true");
  const rendered = view.locator(".markdown-view");
  await expect(rendered.getByRole("heading", { name: "Hello" })).toBeVisible();
  await expect(rendered.getByRole("cell", { name: "2" })).toBeVisible();
  await expect(view.locator(".cm-editor")).toHaveCount(0);
  await page.screenshot({ path: "target/e2e/markdown.png" });

  await page.keyboard.press(`${mod}+Shift+V`);
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(view.locator(".cm-content")).toContainText("# Hello");
  await page.keyboard.press(`${mod}+Shift+V`);
  await expect(rendered).toBeVisible();

  // Ctrl+S saves and shows the text again.
  await page.keyboard.press(`${mod}+s`);
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect(view.getByRole("button", { name: "Save" })).toBeDisabled();
  await expect(view.locator(".cm-content")).toContainText("# Hello");
});

// 14.6: the preview's document style, compared with its reference in each theme. A fixed text
// with every kind of block, so only a change of style changes the picture.
const DOCUMENT = `# Hive document

A paragraph with **bold**, *emphasis*, ~~struck~~, a [link](https://example.com) and \`inline code\`.

## Lists

- [x] A task that is done
- [ ] A task to do
  - a nested item

1. First
2. Second

> A quote: Hive only *observes* agents.

### A table

| Key | Value | Count |
|---|---|--:|
| COVERAGE_EXCLUSIONS.md | Approved coverage exclusions, created when the first one is approved | 1 |
| \`docs/hive.md\` | Single source of truth for every decision | 22 |

#### Code

\`\`\`ts
// Coloured by the editor's parsers.
export function safeUrl(url: string): string {
  return /^https?:/i.test(url) ? url : "";
}
\`\`\`

\`\`\`
plain text, no language
\`\`\`

---

![A screenshot](shot.png)
`;

test("markdown: the preview's document style in light and dark (14.6)", async ({ page }) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "README.md" })
    .click();
  const view = page.getByRole("region", { name: "README.md" });
  await view.locator(".cm-line").first().click();
  await page.keyboard.press(`${mod}+a`);
  await page.keyboard.insertText(DOCUMENT);
  await view.getByRole("button", { name: "Show rendered Markdown" }).click();
  const rendered = view.locator(".markdown-view");
  await expect(rendered.locator(".tok-keyword").first()).toHaveText("export");

  for (const theme of ["dark", "light"]) {
    await page.evaluate((light) => {
      if (light) document.documentElement.dataset.theme = "one-light";
      else delete document.documentElement.dataset.theme;
    }, theme === "light");
    // 20.6: a done task's box is a solid accent, an open one empty; done text recedes.
    const boxes = await rendered.locator(".task-list-item > input").evaluateAll((inputs) =>
      inputs.map((input) => {
        const probe = document.createElement("span");
        probe.style.color = "var(--accent)";
        document.body.append(probe);
        const accent = getComputedStyle(probe).color;
        probe.remove();
        const style = getComputedStyle(input);
        return { accent, background: style.backgroundColor, opacity: style.opacity };
      }),
    );
    expect(boxes, theme).toHaveLength(2);
    const [done, open] = boxes;
    expect(done?.background, theme).toBe(done?.accent);
    expect(open?.background, theme).not.toBe(done?.background);
    expect(open?.background, theme).toBe("rgba(0, 0, 0, 0)");
    expect(done?.opacity, theme).toBe("1");
    await expect(rendered).toHaveScreenshot(`markdown-document-${theme}.png`);
    await expect(rendered.locator(".contains-task-list").first()).toHaveScreenshot(
      `markdown-tasks-${theme}.png`,
    );
  }
});

// 20.1: a table like COVERAGE_EXCLUSIONS.md sizes its columns by content: the date never breaks,
// a path breaks only at a `/` (or not at all), and the long reason takes what is left.
const EXCLUSIONS = `| File | Reason | Approved |
|---|---|---|
| \`src-tauri/src/main.rs\` | ${"Pure wiring. It builds the Tauri app, registers plugins and commands, and runs the event loop, which needs a display and a WebView. ".repeat(2)}| 2026-09-23 |
| \`crates/hive/src/windows*\` | Native-Windows-only code, not compiled on Linux; tested by the Windows CI. | pending |
`;

for (const panelWidth of [280, 640]) {
  test(`markdown: a table's columns fit their content beside a ${panelWidth} px panel (20.1)`, async ({
    page,
  }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "fix-login" }).click();
    await page
      .getByRole("region", { name: "Files" })
      .getByRole("treeitem", { name: "README.md" })
      .click();
    const view = page.getByRole("region", { name: "README.md" });
    await view.locator(".cm-line").first().click();
    await page.keyboard.press(`${mod}+a`);
    await page.keyboard.insertText(EXCLUSIONS);
    await view.getByRole("button", { name: "Show rendered Markdown" }).click();
    const rendered = view.locator(".markdown-view");
    await expect(rendered.getByRole("table")).toBeVisible();

    const layout = await rendered.evaluate((root) => {
      // How many lines a piece of a cell's text takes.
      const lines = (node: Node, start: number, end: number) => {
        const range = document.createRange();
        range.setStart(node, start);
        range.setEnd(node, end);
        return new Set([...range.getClientRects()].map((r) => Math.round(r.top))).size;
      };
      const text = (cell: Element) => {
        const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
        return walker.nextNode() as Text;
      };
      const rows = [...root.querySelectorAll("tbody tr")];
      return {
        dates: rows.map((tr) => {
          const node = text(tr.children[2] as Element);
          return lines(node, 0, node.length);
        }),
        // Each piece of a path between slashes stays on one line.
        paths: rows.flatMap((tr) => {
          const node = text(tr.children[0] as Element);
          let at = 0;
          return node.data.split("/").map((part) => {
            const n = lines(node, at, at + part.length);
            at += part.length + 1;
            return n;
          });
        }),
        // The reason column is wider than the file column.
        widths: [...(rows[0]?.children ?? [])].map((td) => (td as HTMLElement).offsetWidth),
        pageFits: document.documentElement.scrollWidth <= document.documentElement.clientWidth,
        viewFits: root.scrollWidth <= root.clientWidth,
      };
    });
    await page.screenshot({ path: `target/e2e/markdown-columns-${panelWidth}.png` });
    expect(layout.dates).toEqual([1, 1]);
    expect(layout.paths.every((n) => n === 1)).toBe(true);
    const [file = 0, reason = 0] = layout.widths;
    expect(reason).toBeGreaterThan(file);
    expect(layout.pageFits).toBe(true);
    expect(layout.viewFits).toBe(true);
    if (panelWidth === 640) await expect(rendered).toHaveScreenshot("markdown-table-columns.png");
  });
}

// 13.1: the rendered Markdown never scrolls sideways; a wide table, a long inline path and a long
// code line wrap, in the editor area beside the right panel at its narrowest and its widest.
for (const panelWidth of [280, 640]) {
  test(`markdown: nothing in the preview scrolls sideways beside a ${panelWidth} px panel (13.1)`, async ({
    page,
  }) => {
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "fix-login" }).click();
    const panel = page.getByRole("region", { name: "Files" });
    await panel.getByRole("treeitem", { name: "docs" }).click();
    await panel.getByRole("treeitem", { name: "api.md" }).click();
    const view = page.getByRole("region", { name: "docs/api.md" });
    await view.getByRole("button", { name: "Show rendered Markdown" }).click();
    const rendered = view.locator(".markdown-view");
    await expect(rendered.getByRole("table")).toBeVisible();
    await expect(rendered.locator("p code")).toBeVisible();
    await expect(rendered.locator("pre")).toBeVisible();

    // 20.1: a table whose columns cannot narrow to fit (one-word paths) scrolls inside its own
    // box, which stays within the view; nothing else scrolls.
    const fit = await rendered.evaluate((root) => {
      const right = root.getBoundingClientRect().right;
      return [root, ...root.querySelectorAll(".md-table, p, pre")].map((el) => ({
        el: el.tagName,
        fits:
          (el.matches(".md-table") || el.scrollWidth <= el.clientWidth) &&
          el.getBoundingClientRect().right <= right,
      }));
    });
    expect(fit).toEqual(["DIV", "DIV", "P", "PRE"].map((el) => ({ el, fits: true })));
    const table = rendered.locator(".md-table");
    expect(await table.evaluate((el) => getComputedStyle(el).overflowX)).toBe("auto");
    await page.screenshot({ path: `target/e2e/markdown-wrap-${panelWidth}.png` });
  });
}

// 15.5: the document column is centred in a view wider than it (equal gaps on both sides) and
// fills a narrower one (no gaps), in light and dark. The default window leaves the editor area
// just under the column's 900 px, so the wide view is a wider window.
for (const [view, windowWidth, panelWidth] of [
  ["wide", 1920, 280],
  ["narrow", 1440, 640],
] as const) {
  test(`markdown: the preview's column in a ${view} view, light and dark (15.5)`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: windowWidth, height: 900 });
    await page.addInitScript(
      (w) => localStorage.setItem("hive.widths", JSON.stringify({ panelWidth: w })),
      panelWidth,
    );
    await page.goto("/");
    const tree = page.getByRole("navigation", { name: "Projects" });
    await tree.getByRole("button", { name: "fix-login" }).click();
    const panel = page.getByRole("region", { name: "Files" });
    await panel.getByRole("treeitem", { name: "docs" }).click();
    await panel.getByRole("treeitem", { name: "api.md" }).click();
    const tab = page.getByRole("region", { name: "docs/api.md" });
    await tab.getByRole("button", { name: "Show rendered Markdown" }).click();
    const rendered = tab.locator(".markdown-view");
    await expect(rendered.getByRole("table")).toBeVisible();

    for (const theme of ["dark", "light"]) {
      await page.evaluate((light) => {
        if (light) document.documentElement.dataset.theme = "one-light";
        else delete document.documentElement.dataset.theme;
      }, theme === "light");
      // The gaps between the column and the view's content box (inside its padding).
      const { left, right, room } = await rendered.evaluate((root) => {
        const style = getComputedStyle(root);
        const origin = root.getBoundingClientRect().left + root.clientLeft;
        const start = origin + Number.parseFloat(style.paddingLeft);
        const end = origin + root.clientWidth - Number.parseFloat(style.paddingRight);
        const doc = root.querySelector(".markdown-doc")?.getBoundingClientRect();
        return {
          left: (doc?.left ?? 0) - start,
          right: end - (doc?.right ?? 0),
          room: end - start,
        };
      });
      if (view === "wide") {
        expect(room, theme).toBeGreaterThan(900);
        expect(left, theme).toBeGreaterThan(0);
        expect(left, theme).toBeCloseTo(right, 0);
      } else {
        expect(room, theme).toBeLessThan(900);
        expect([left, right], theme).toEqual([0, 0]);
      }
    }
    await page.screenshot({ path: `target/e2e/markdown-centred-${view}.png` });
  });
}
