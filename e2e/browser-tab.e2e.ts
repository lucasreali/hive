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
