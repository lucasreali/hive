import { expect, type Page, test } from "@playwright/test";

/** The text of a terminal's buffer, read from the page's own terminal manager. */
function screen(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(id).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  }, id);
}

/** The terminal the service was last told is in view. */
const inView = (page: Page) => page.evaluate(() => (window as unknown as { view: unknown }).view);

test("split: Ctrl+Shift+D shows two terminals side by side, a click focuses one, again un-splits", async ({
  page,
}) => {
  await page.goto("/");
  // Records what the app tells the service is in view.
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const setView = transport.setView.bind(transport);
    transport.setView = (terminal: number | null, focused: boolean) => {
      Object.assign(window, { view: terminal });
      return setView(terminal, focused);
    };
  });
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page, 1)).toBe("mock$");

  // Alone in its worktree, the terminal splits beside a new one there, which takes the focus.
  await page.keyboard.press("Control+Shift+D");
  await expect(tabs.getByRole("tab")).toHaveCount(2);
  const panes = page.locator(".terminal-pane:visible");
  await expect(panes).toHaveCount(2);
  // Both render with WebGL, side by side, each fitted to its half.
  await expect(page.locator(".terminal-pane:visible canvas").first()).toBeAttached();
  await expect(page.locator('.terminal-pane[data-pane="left"] canvas').first()).toBeAttached();
  await expect(page.locator('.terminal-pane[data-pane="right"] canvas').first()).toBeAttached();
  const left = page.locator('.terminal-pane[data-pane="left"]');
  const right = page.locator('.terminal-pane[data-pane="right"]');
  const [l, r] = [await left.boundingBox(), await right.boundingBox()];
  expect(l && r && l.x + l.width < r.x).toBe(true);
  await expect.poll(() => screen(page, 2)).toBe("mock$");
  await expect.poll(() => inView(page)).toBe(2);
  await page.keyboard.type("right");
  await expect.poll(() => screen(page, 2)).toBe("mock$ right");

  // A click in the left pane focuses it: keys go there and it is the one in view.
  await left.click();
  await expect.poll(() => inView(page)).toBe(1);
  await page.keyboard.type("left");
  await expect.poll(() => screen(page, 1)).toBe("mock$ left");
  await expect(tabs.getByRole("tab").first()).toHaveAttribute("aria-selected", "true");

  // Dragging the divider resizes both panes.
  const divider = page.getByRole("separator", { name: "Resize the split terminals" });
  const before = (await left.boundingBox())?.width ?? 0;
  const box = await divider.boundingBox();
  if (!box) throw new Error("no divider");
  await page.mouse.move(box.x + box.width / 2, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(box.x - 150, box.y + 100, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await left.boundingBox())?.width ?? 0).toBeLessThan(before - 100);

  // Pressed again, the split ends; the focused terminal stays shown.
  await page.keyboard.press("Control+Shift+D");
  await expect(panes).toHaveCount(1);
  await expect(divider).toHaveCount(0);
  await expect(page.locator(".terminal-pane:visible canvas").first()).toBeAttached();

  // From the tab menu: Split right uses the next tab of the worktree; closing a pane un-splits.
  await tabs.getByRole("tab").first().click({ button: "right" });
  await page.getByRole("menuitem", { name: "Split right" }).click();
  await expect(panes).toHaveCount(2);
  await expect(tabs.getByRole("tab")).toHaveCount(2);
  await expect.poll(() => inView(page)).toBe(2);
  await page.getByRole("button", { name: "Close terminal fix-login" }).last().click();
  await expect(panes).toHaveCount(1);
  await expect(tabs.getByRole("tab")).toHaveCount(1);
  await expect.poll(() => inView(page)).toBe(1);
});
