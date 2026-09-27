import { expect, type Page, test } from "@playwright/test";

/** The names of the tab bar's tabs, left to right. */
const names = (page: Page) =>
  page
    .getByRole("tablist", { name: "Open terminals and files" })
    .locator(".tab-name")
    .allTextContents();

/** Opens `name` from the Files tree of the selected worktree, as editable text. */
async function openFile(page: Page, name: string) {
  const panel = page.getByRole("region", { name: "Files" });
  await panel.getByRole("treeitem", { name, exact: true }).click();
  const view = page.getByRole("region", { name });
  await expect(view.locator(".cm-content")).not.toBeEmpty();
  return view;
}

test("file tabs: each file keeps its edits; tabs reorder by drag, remembered after a reload", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();

  // Two files, each in its own tab, each with its own unsaved edits.
  const readme = await openFile(page, "README.md");
  await readme.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// readme\n");
  const pkg = await openFile(page, "package.json");
  await pkg.locator(".cm-line").first().click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("// pkg\n");
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json"]);
  await page.getByRole("tab", { name: "README.md" }).click();
  await expect(readme.locator(".cm-line").first()).toHaveText("// readme");
  await page.getByRole("tab", { name: "package.json" }).click();
  await expect(pkg.locator(".cm-line").first()).toHaveText("// pkg");
  await expect(page.getByRole("button", { name: /\(unsaved changes\)$/ })).toHaveCount(2);

  // A new terminal goes last; dragged onto the left edge of README.md it lands first.
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json", "fix-login"]);
  const tab = (name: string) => page.locator(".tab", { hasText: name });
  await tab("fix-login").dragTo(tab("README.md"), { targetPosition: { x: 2, y: 10 } });
  await expect.poll(() => names(page)).toEqual(["fix-login", "README.md", "package.json"]);
  // package.json before README.md.
  await tab("package.json").dragTo(tab("README.md"), { targetPosition: { x: 2, y: 10 } });
  await expect.poll(() => names(page)).toEqual(["fix-login", "package.json", "README.md"]);
  await expect(page.locator("[data-drop]")).toHaveCount(0);

  // After a reload the files keep their order, whatever order they open in.
  await page.reload();
  await tree.getByRole("button", { name: "fix-login" }).click();
  await openFile(page, "README.md");
  await openFile(page, "package.json");
  await expect.poll(() => names(page)).toEqual(["package.json", "README.md"]);
});

test("tab drag: one accent line shows where the tab lands, and goes away after", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await openFile(page, "README.md");
  await openFile(page, "package.json");
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json", "fix-login"]);
  const tab = (name: string) => page.locator(".tab", { hasText: name });
  const box = async (name: string) => {
    const b = await tab(name).boundingBox();
    if (!b) throw new Error(`no tab ${name}`);
    return b;
  };
  /** The drop line (left x, size, colour), or how many tabs show one when not exactly one. */
  const line = () =>
    page.evaluate(() => {
      const tabs = document.querySelectorAll(".tab[data-drop]");
      const tab = tabs[0];
      if (tabs.length !== 1 || !tab) return tabs.length;
      const style = getComputedStyle(tab, "::after");
      const at = tab.getBoundingClientRect();
      const width = Number.parseFloat(style.width);
      const left =
        tab.getAttribute("data-drop") === "after"
          ? at.right - Number.parseFloat(style.right) - width
          : at.left + Number.parseFloat(style.left);
      return { left, width, height: style.height, color: style.backgroundColor };
    });
  const accent = await page.evaluate(() => {
    const probe = document.createElement("i");
    probe.style.color = "var(--accent)";
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
  /** The centre of `part` of the tab `name`. */
  const at = async (name: string, part: string) => {
    const b = await tab(name).locator(part).first().boundingBox();
    if (!b) throw new Error(`no ${part} in ${name}`);
    return [b.x + b.width / 2, b.y + b.height / 2] as const;
  };
  /** Picks up the tab `name` by its label. */
  const pickUp = async (name: string) => {
    await page.mouse.move(...(await at(name, ".tab-name")));
    await page.mouse.down();
  };
  // Chromium fires dragover on a newly entered element only at the drag's next update, which a
  // real drag sends every 50 ms: `move` goes there, then updates once more.
  const move = async (x: number, y: number) => {
    await page.mouse.move(x, y, { steps: 4 });
    await page.mouse.move(x, y);
  };

  // Picked up, fix-login is dimmed; over README.md's close button, on its right half, the line
  // is on README.md's right edge.
  await pickUp("fix-login");
  const readme = await box("README.md");
  const [cx, cy] = await at("README.md", ".tab-close");
  await move(cx, cy);
  await expect(tab("fix-login")).toHaveCSS("opacity", "0.5");
  const edge = { left: readme.x + readme.width - 1, width: 2, height: `${readme.height}px` };
  await expect.poll(line).toEqual({ ...edge, color: accent });
  // Onto README.md's name (still its right half), then over the border onto package.json's
  // padding, icon and name (its left half): the same line stays at every step, once.
  const pkg = await box("package.json");
  const steps: [number, number][] = [
    [readme.x + readme.width - 40, cy],
    [pkg.x + 4, cy],
    [pkg.x + 4, cy + 1],
    [(await at("package.json", ".tab-label > svg"))[0], cy],
    [pkg.x + 40, cy],
  ];
  for (const [x, y] of steps) {
    await page.mouse.move(x, y);
    expect(await line()).toEqual({ ...edge, color: accent });
  }
  await expect(tab("package.json")).toHaveAttribute("data-drop", "before");
  // Dropped outside the bar: nothing moves, no line, no dimming.
  await move(pkg.x + 20, pkg.y + 300);
  await expect.poll(line).toBe(0);
  await page.mouse.up();
  await expect.poll(() => names(page)).toEqual(["README.md", "package.json", "fix-login"]);
  await expect(page.locator("[data-drop], [data-dragging]")).toHaveCount(0);

  // Past the last tab, on the empty bar: the line after fix-login, and README.md lands last.
  await pickUp("README.md");
  const last = await box("fix-login");
  await move(last.x + last.width + 200, cy);
  await expect(tab("fix-login")).toHaveAttribute("data-drop", "after");
  expect(await line()).toMatchObject({ left: last.x + last.width - 1 });
  await page.mouse.up();
  await expect.poll(() => names(page)).toEqual(["package.json", "fix-login", "README.md"]);
  await expect(page.locator("[data-drop], [data-dragging]")).toHaveCount(0);

  // Esc cancels the drag: no line remains, nothing moves.
  await pickUp("README.md");
  const first = await box("package.json");
  await move(first.x + 5, cy);
  await expect(tab("package.json")).toHaveAttribute("data-drop", "before");
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect.poll(() => names(page)).toEqual(["package.json", "fix-login", "README.md"]);
  await expect(page.locator("[data-drop], [data-dragging]")).toHaveCount(0);
});
