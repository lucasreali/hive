import { expect, test } from "@playwright/test";

test("media: an image and a PDF show as themselves in the viewer (24.2, 24.3)", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });

  // A changed image shows as the image, not "Binary file not shown." nor a diff.
  await tree.getByRole("button", { name: "refactor-auth" }).click();
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Diff" }).click();
  const changes = panel.getByRole("tree", { name: "Files" });
  await changes
    .getByRole("treeitem")
    .filter({ has: page.getByText("assets", { exact: true }) })
    .click();
  await changes.getByRole("treeitem", { name: /logo\.png/ }).click();
  const view = page.getByRole("region", { name: "assets/logo.png" });
  const image = view.getByRole("img", { name: "assets/logo.png" });
  await expect(image).toBeVisible();
  await expect.poll(() => image.evaluate((e: HTMLImageElement) => e.naturalWidth)).toBe(24);
  await expect(view).not.toContainText("Binary file not shown.");
  await expect(view.locator(".cm-editor")).toHaveCount(0);

  // A PDF an agent writes shows in a plain frame (the WebView draws none in a sandboxed one).
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page.keyboard.type("touch guide.pdf");
  await page.keyboard.press("Enter");
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Files" }).click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "guide.pdf" })
    .click();
  const frame = page.getByRole("region", { name: "guide.pdf" }).locator("iframe.media-view");
  await expect(frame).toBeVisible();
  await expect(frame).toHaveAttribute("src", /^data:application\/pdf;base64,/);
  await expect(frame).not.toHaveAttribute("sandbox");
  await page.screenshot({ path: "target/e2e/media.png" });
});
