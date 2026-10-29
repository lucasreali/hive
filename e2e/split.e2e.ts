import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

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
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page, 1)).toBe("mock$");

  // Alone in its worktree, the terminal splits beside a new one there, which takes the focus.
  await page.keyboard.press(`${mod}+Shift+D`);
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
  await page.keyboard.press(`${mod}+Shift+D`);
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

/** The terminals shown, left to right, as the store has them. */
const shown = (page: Page) =>
  page.evaluate(async () => {
    const [store, tabs] = ["/src/store.ts", "/src/tabs.ts"];
    const { useHive } = await import(/* @vite-ignore */ store);
    const { shownTerminals } = await import(/* @vite-ignore */ tabs);
    return shownTerminals(useHive.getState());
  });

test("split pairs (20.5): two in a worktree, seen in the bar, rearranged by dragging and by the menu", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const tab = (i: number) => tabs.getByRole("tab").nth(i);
  const pairs = () =>
    tabs.locator(".tab").evaluateAll((els) => els.map((el) => el.getAttribute("data-pair")));
  await tree.getByRole("button", { name: "fix-login" }).click();
  for (let i = 1; i <= 4; i++) {
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
    await expect(tabs.getByRole("tab")).toHaveCount(i);
  }
  await expect.poll(() => screen(page, 4)).toBe("mock$");

  // Two pairs: 1 with 2, 3 with 4; a click on a tab of either shows its pair.
  await tab(0).click();
  await page.keyboard.press(`${mod}+Shift+D`);
  await tab(2).click();
  await page.keyboard.press(`${mod}+Shift+D`);
  await expect.poll(() => shown(page)).toEqual([3, 4]);
  await expect.poll(pairs).toEqual(["left", "right", "left", "right"]);
  await tab(0).click();
  await expect.poll(() => shown(page)).toEqual([1, 2]);
  await expect(page.locator(".terminal-pane:visible")).toHaveCount(2);
  await tab(3).click();
  await expect.poll(() => shown(page)).toEqual([3, 4]);
  await expect(tab(0)).toHaveAttribute("aria-description", "Split with fix-login");
  // The bar with a shown pair (3, 4) and a hidden one (1, 2).
  await expect(page.locator(".terminals > .bar")).toHaveScreenshot("split-pairs-bar.png");

  // Tab 1 dragged onto the right pane takes it: 4 stays alone, and 2 too, its pair ended.
  await tab(0).dragTo(page.locator('.terminal-pane[data-pane="right"]:visible'));
  await expect.poll(() => shown(page)).toEqual([3, 1]);
  await expect.poll(pairs).toEqual([null, "left", "right", null]);
  await expect(page.locator(".pane-drop")).toHaveCount(0);

  // Swap sides, from the tab menu.
  await tab(1).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Swap sides" }).click();
  await expect.poll(() => shown(page)).toEqual([1, 3]);

  // Tab 2 dropped on the left half of terminal 4, shown alone: a pair, 2 on the left.
  await tab(3).click();
  await expect.poll(() => shown(page)).toEqual([4]);
  await tab(0).dragTo(page.locator(".terminal-host"), { targetPosition: { x: 40, y: 100 } });
  await expect.poll(() => shown(page)).toEqual([2, 4]);
  await expect.poll(pairs).toEqual(["left", "right", "left", "right"]);
});
