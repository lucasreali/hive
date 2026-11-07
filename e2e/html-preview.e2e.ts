import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

// The CSP the app's scheme gives an HTML page on Windows, whose scheme URL the mock gives,
// read from its one place (`scheme::policy`).
const scheme = readFileSync(new URL("../src-tauri/src/scheme.rs", import.meta.url), "utf8");
const policy = scheme.match(/const HTML_POLICY_WINDOWS: &str =\s*"([^"]+)";/)?.[1] ?? "";

const PAGE = `<!doctype html><title>Page</title>
<img id="dot" src="img/dot.png" alt="dot">
<p id="origin"></p><p id="net">pending</p>
<script>
  document.getElementById("origin").textContent = self.origin;
  const net = document.getElementById("net");
  fetch("https://example.com/data").then(() => { net.textContent = "reached"; }, () => { net.textContent = "blocked"; });
</script>`;
const DOT = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

// 24.4: an HTML file's Preview on the fake service, its scheme answers played by routes with
// the app's CSP: its relative image loads, its inline script runs in origin `null`, and a
// `fetch` to the network fails though the host would answer it.
test("html preview: a page shows with its relative image, sandboxed and off the network", async ({
  page,
}) => {
  expect(policy).toContain("connect-src 'none'");
  let reached = 0;
  await page.route("https://example.com/**", (route) => {
    reached++;
    return route.fulfill({ body: "{}", headers: { "Access-Control-Allow-Origin": "*" } });
  });
  await page.route("http://hive-file.localhost/mock/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/mock/page.html") {
      const headers = { "Content-Security-Policy": policy, "X-Content-Type-Options": "nosniff" };
      return route.fulfill({ body: PAGE, contentType: "text/html; charset=utf-8", headers });
    }
    if (path === "/mock/img/dot.png") return route.fulfill({ body: DOT, contentType: "image/png" });
    return route.fulfill({ status: 404 });
  });

  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await page.keyboard.type("touch page.html");
  await page.keyboard.press("Enter");
  const panel = page.getByRole("complementary", { name: "Side panel" });
  await panel.getByRole("tablist", { name: "Panel" }).getByRole("tab", { name: "Files" }).click();
  await page
    .getByRole("region", { name: "Files" })
    .getByRole("treeitem", { name: "page.html" })
    .click();
  const view = page.getByRole("region", { name: "page.html" });
  // Its text first, then its Preview.
  await expect(view.locator(".cm-editor")).toBeVisible();
  await view.getByRole("button", { name: "Preview the page" }).click();
  const frame = view.locator("iframe.page-frame");
  await expect(frame).toHaveAttribute("sandbox", "allow-scripts");
  const inside = view.frameLocator("iframe.page-frame");
  const dot = inside.getByRole("img", { name: "dot" });
  await expect.poll(() => dot.evaluate((e: HTMLImageElement) => e.naturalWidth)).toBe(1);
  await expect(inside.locator("#origin")).toHaveText("null");
  await expect(inside.locator("#net")).toHaveText("blocked");
  expect(reached).toBe(0);
  await page.screenshot({ path: "target/e2e/html-preview.png" });
});
