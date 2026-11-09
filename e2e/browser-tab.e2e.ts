import { expect, test } from "@playwright/test";

/** Turns the agent's browser on in the mock, and starts an agent in fix-login. */
async function agent(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.evaluate(async () => {
    const url = "/src/transport/index.ts";
    const { transport } = await import(/* @vite-ignore */ url);
    const store = "/src/store.ts";
    const { useHive } = await import(/* @vite-ignore */ store);
    const settings = useHive.getState().settings;
    await transport.setSettings({
      ...settings,
      browser: { ...settings.browser, enabled: true },
    });
    const watch = transport.watchBrowser.bind(transport);
    Object.assign(window, { watched: [] });
    transport.watchBrowser = (worktree: string | null) => {
      (window as unknown as { watched: (string | null)[] }).watched.push(worktree);
      return watch(worktree);
    };
  });
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(page.locator(".xterm-helper-textarea").first()).toBeFocused();
  await page.keyboard.type("claude\n");
  await expect(
    page.getByRole("navigation", { name: "Projects" }).locator(".tree-row.agent"),
  ).toBeVisible();
}

/** Gives the shown browser a frame `width` pixels wide; that width. */
const frameOf = (page: import("@playwright/test").Page, width: number) =>
  page.evaluate(async (width) => {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = 100;
    const data = canvas.toDataURL("image/jpeg").split(",")[1];
    const reduce = "/src/reduce.ts";
    const { apply } = await import(/* @vite-ignore */ reduce);
    const store = "/src/store.ts";
    const { useHive } = await import(/* @vite-ignore */ store);
    apply({ type: "browser_frame", worktree: useHive.getState().browserShown, data });
    return width;
  }, width);

const call = async (page: import("@playwright/test").Page, line: string) => {
  await page.locator(".xterm-helper-textarea").first().focus();
  await page.keyboard.type(`${line}\n`);
};

test("an agent's browser calls open the tab unfocused; the frame fits it; closed, it stays closed", async ({
  page,
}) => {
  await agent(page);
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const tab = tabs.getByRole("tab", { name: "Mock shop" });
  await expect(tab).toHaveCount(0);
  const view = page.getByRole("region", { name: "Agent's browser" });
  const watched = () => page.evaluate(() => (window as unknown as { watched: unknown[] }).watched);

  await call(page, "browser-call clicked 'Learn more'");
  await expect(tab).toBeVisible();
  // Unfocused: the terminal stays in front, and no screencast runs.
  await expect(tab).toHaveAttribute("aria-selected", "false");
  await expect(view).toHaveCount(0);
  expect(await watched()).toEqual([]);

  // The sign on the agent: its click shows the tab.
  const globe = page.getByRole("button", { name: "Show the agent's browser" });
  await expect(globe).toBeVisible();
  await globe.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  // Shown now, so the spy has recorded the screencast's start: its empty list above meant none.
  await expect
    .poll(async () => (await watched()).at(-1))
    .toBe("/home/user/projects/shop/.claude/worktrees/fix-login");
  await expect(view.locator(".browser-caption")).toHaveText("Claude · clicked 'Learn more'");
  await expect(view.locator(".browser-url")).toHaveText("http://localhost:5173/");
  await expect(page.getByRole("button", { name: "Open in my browser" })).toBeEnabled();

  // A small frame stays at its own size; a large one fits the tab's width.
  const img = view.getByRole("img", { name: "The agent's browser" });
  await expect(img).toBeVisible();
  const natural = await img.evaluate((el: HTMLImageElement) => el.naturalWidth);
  expect((await img.boundingBox())?.width).toBe(natural);
  const pageWidth = (await view.locator(".browser-page").boundingBox())?.width as number;
  const wide = await page.evaluate(
    async (width) => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = 100;
      const data = canvas.toDataURL("image/jpeg").split(",")[1];
      const url = "/src/reduce.ts";
      const { apply } = await import(/* @vite-ignore */ url);
      const store = "/src/store.ts";
      const { useHive } = await import(/* @vite-ignore */ store);
      apply({ type: "browser_frame", worktree: useHive.getState().browserShown, data });
      return width;
    },
    Math.ceil(pageWidth * 3),
  );
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(wide);
  expect(Math.round((await img.boundingBox())?.width as number)).toBeLessThanOrEqual(
    Math.ceil(pageWidth),
  );

  // Closed by the human: the next call does not bring it back, the globe does.
  await tabs.getByRole("button", { name: "Close browser Mock shop" }).click();
  await expect(tab).toHaveCount(0);
  await call(page, "browser-call typed 'Honey bee' and pressed Enter");
  await expect(globe).toBeVisible();
  await expect(tab).toHaveCount(0);
  await globe.click();
  await expect(view.locator(".browser-caption")).toHaveText(
    "Claude · typed 'Honey bee' and pressed Enter",
  );
});

test("stopped, the tab stays with its page and says so; the panel has no Browser view", async ({
  page,
}) => {
  await agent(page);
  await call(page, "browser-call opened http://localhost:5173/");
  const tab = page
    .getByRole("tablist", { name: "Open terminals and files" })
    .getByRole("tab", { name: "Mock shop" });
  await expect(tab).toBeVisible();
  await call(page, "browser-stop");
  await tab.click();
  const view = page.getByRole("region", { name: "Agent's browser" });
  await expect(view).toContainText("Browser closed");
  await expect(view.locator(".browser-url")).toHaveText("http://localhost:5173/");
  await expect(view.getByRole("img", { name: "The agent's browser" })).toBeVisible();

  const panel = page.getByRole("tablist", { name: "Panel" });
  await expect(panel.getByRole("tab", { name: "Files" })).toBeVisible();
  await expect(panel.getByRole("tab", { name: "Browser" })).toHaveCount(0);
});

test("the tab's menu shows it beside the agent's terminal, both in view", async ({ page }) => {
  await agent(page);
  await call(page, "browser-call clicked 'Learn more'");
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  const tab = tabs.getByRole("tab", { name: "Mock shop" });
  await expect(tab).toBeVisible();
  // Never by itself.
  const view = page.getByRole("region", { name: "Agent's browser" });
  await expect(view).toHaveCount(0);
  await tab.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Show beside the agent" }).click();
  await expect(view).toBeVisible();
  const terminal = page.locator(".terminal-host");
  await expect(terminal).toBeVisible();
  const [left, right] = [await terminal.boundingBox(), await view.boundingBox()];
  expect((left?.x as number) + (left?.width as number)).toBeLessThanOrEqual(
    (right?.x as number) + 1,
  );
  // A frame wider than its half is scaled to fit it.
  const img = view.getByRole("img", { name: "The agent's browser" });
  const wide = await frameOf(page, Math.ceil((right?.width as number) * 2));
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(wide);
  const shown = (await img.boundingBox())?.width as number;
  expect(shown).toBeLessThanOrEqual(right?.width as number);
  expect(shown).toBeLessThan(wide);
});

test("a very long address stays cut on its tab: its close button and the bar stay in reach", async ({
  page,
}) => {
  await agent(page);
  await call(page, "browser-call clicked 'Learn more'");
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await expect(tabs.getByRole("tab", { name: "Mock shop" })).toBeVisible();
  const long = `https://a.test/${"x".repeat(2031)}…`;
  await page.evaluate(async (url) => {
    const reduce = "/src/reduce.ts";
    const { apply } = await import(/* @vite-ignore */ reduce);
    const store = "/src/store.ts";
    const { useHive } = await import(/* @vite-ignore */ store);
    const { browserTabs } = useHive.getState();
    apply({
      type: "browser_status",
      worktree: browserTabs[0].browser,
      running: true,
      url,
      title: null,
      error: null,
    });
  }, long);
  const name = tabs.locator(".tab-name[data-browser]");
  await expect(name).toHaveText(long);
  expect((await name.boundingBox())?.width as number).toBeLessThanOrEqual(220);
  // Its × and the "+" after it are inside the window, and the × still closes it.
  const close = tabs.locator(".tab:has(.tab-name[data-browser]) .tab-close");
  const width = page.viewportSize()?.width as number;
  const box = await close.boundingBox();
  expect((box?.x as number) + (box?.width as number)).toBeLessThanOrEqual(width);
  const plus = await page.getByTitle("New terminal or file").boundingBox();
  expect((plus?.x as number) + (plus?.width as number)).toBeLessThanOrEqual(width);
  await close.hover();
  await close.click();
  await expect(name).toHaveCount(0);
});
