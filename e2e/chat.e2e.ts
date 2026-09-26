import { expect, test } from "@playwright/test";

test("chat: confirm the folder, send, entries arrive, a tool opens, Stop while busy, close", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const tabs = page.getByRole("tablist", { name: "Open terminals and files" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();

  // The first chat in a folder asks first.
  const dialog = page.getByRole("dialog", { name: "Chat in this folder?" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();

  const chat = page.getByRole("region", { name: "Chat" });
  await expect(chat.getByText("Send a message to start.")).toBeVisible();
  await expect(tabs.getByRole("tab", { name: "fix-login" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const input = chat.getByRole("textbox", { name: "Message" });
  await input.fill("list the files");
  await input.press("Shift+Enter");
  await input.press("Enter");
  await expect(input).toHaveValue("");
  // While the turn runs, Send turns into Stop.
  await expect(chat.getByRole("button", { name: "Stop" })).toBeVisible();
  // Live text (7.3h): the reply shows as it grows (checked every frame: it lasts one step).
  await page.waitForFunction(() =>
    [...document.querySelectorAll('[data-role="assistant"]')].some(
      (row) => row.textContent === "ClaudeThe worktree has",
    ),
  );
  const entries = chat.locator(".transcript-entry");
  await expect(entries.first()).toHaveText("Youlist the files");
  const tool = chat.locator('.transcript-entry[data-role="tool"]');
  await expect(tool).toHaveAttribute("data-status", "ok");
  // Live text grows in place, then the turn ends with its usage.
  await expect(chat.locator('[data-role="assistant"]').last()).toContainText(
    "a README.md and a src folder.",
  );
  await expect(chat.locator('[data-role="usage"]')).toBeVisible();
  await expect(chat.getByRole("button", { name: "Send" })).toBeVisible();
  // Thinking is collapsed; a finished tool opens to its output.
  await expect(chat.locator('[data-role="thinking"] details')).not.toHaveAttribute("open");
  await expect(tool.locator(".tool-output")).toBeHidden();
  await tool.locator("summary").click();
  await expect(tool.locator(".tool-output")).toHaveText("README.md\nsrc");

  // Stop ends a running turn.
  await input.fill("again");
  await input.press("Enter");
  await chat.getByRole("button", { name: "Stop" }).click();
  await expect(chat.locator('[data-role="note"]')).toHaveText("Interrupted");
  await expect(chat.getByRole("button", { name: "Send" })).toBeVisible();

  await page.getByRole("button", { name: "Close chat fix-login" }).click();
  await expect(chat).toBeHidden();
  await expect(tabs.getByRole("tab")).toHaveCount(0);
});

/** A 1×1 PNG. */
const DOT =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

test("chat: a pasted image shows as a thumbnail, is sent, and opens larger", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await expect(page.getByRole("dialog", { name: "Chat in this folder?" })).toBeVisible();
  await page.keyboard.press("Enter");
  const chat = page.getByRole("region", { name: "Chat" });
  const input = chat.getByRole("textbox", { name: "Message" });
  await expect(input).toBeEnabled();

  await input.evaluate((element, dot) => {
    const bytes = Uint8Array.from(atob(dot), (c) => c.charCodeAt(0));
    const clipboardData = new DataTransfer();
    clipboardData.items.add(new File([bytes], "dot.png", { type: "image/png" }));
    const paste = new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true });
    element.dispatchEvent(paste);
  }, DOT);
  const thumbs = chat.getByRole("list", { name: "Images to send" }).getByRole("img");
  await expect(thumbs).toHaveAttribute("src", `data:image/png;base64,${DOT}`);

  // One box (7.16): thumbnails, message and toolbar inside it; mode and Send on one row.
  const box = await chat.locator("form.chat-composer").boundingBox();
  const inner = await Promise.all(
    [
      thumbs,
      input,
      chat.getByRole("button", { name: "Attach image" }),
      chat.getByRole("combobox", { name: "Permission mode" }),
      chat.getByRole("button", { name: "Send" }),
    ].map((part) => part.boundingBox()),
  );
  for (const part of inner) {
    expect(part?.x).toBeGreaterThanOrEqual(box?.x as number);
    expect(part?.y).toBeGreaterThanOrEqual(box?.y as number);
    expect((part?.x as number) + (part?.width as number)).toBeLessThanOrEqual(
      (box?.x as number) + (box?.width as number),
    );
    expect((part?.y as number) + (part?.height as number)).toBeLessThanOrEqual(
      (box?.y as number) + (box?.height as number),
    );
  }
  const [, message, attach, mode, send] = inner;
  const middle = (b: typeof box) => (b?.y as number) + (b?.height as number) / 2;
  expect(Math.abs(middle(mode) - middle(send))).toBeLessThan(2);
  expect(Math.abs(middle(attach) - middle(send))).toBeLessThan(2);
  expect(mode?.y).toBeGreaterThan((message?.y as number) + (message?.height as number) - 1);
  expect(send?.x).toBeGreaterThan(mode?.x as number);

  await input.fill("what is this?");
  await input.press("Enter");
  await expect(chat.getByRole("list", { name: "Images to send" })).toBeHidden();

  const sent = chat.locator('.transcript-entry[data-role="user"]').first();
  await expect(sent).toContainText("what is this?");
  const image = sent.getByTitle("Enlarge the image");
  await expect(image.locator("img")).toHaveAttribute("src", `data:image/png;base64,${DOT}`);
  await image.click();
  await expect(sent.getByTitle("Shrink the image")).toHaveAttribute("aria-pressed", "true");

  // Attach image opens a picker; its image joins the next message.
  const chooser = page.waitForEvent("filechooser");
  await chat.getByRole("button", { name: "Attach image" }).click();
  await (await chooser).setFiles({
    name: "dot.png",
    mimeType: "image/png",
    buffer: Buffer.from(DOT, "base64"),
  });
  await expect(thumbs).toHaveAttribute("src", `data:image/png;base64,${DOT}`);
});

test("chat: permission, question and plan cards pin above the composer and answer by keyboard", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await page.getByRole("button", { name: "Start chat" }).click();
  const chat = page.getByRole("region", { name: "Chat" });
  const input = chat.getByRole("textbox", { name: "Message", exact: true });
  await expect(input).toBeEnabled();
  const reply = chat.locator('[data-role="assistant"]').last();

  // A permission leaves the focus in the composer (8.10): typing goes on there. Once the
  // user clicks the card, Enter allows it and the card goes.
  await input.fill("ask permission to clean");
  await input.press("Enter");
  const permission = chat.getByRole("region", { name: "Permission request" });
  await expect(permission.locator("pre")).toHaveText("rm -rf target");
  await expect(input).toBeFocused();
  await page.keyboard.type("draft");
  await expect(input).toHaveValue("draft");
  await permission.locator("pre").click();
  await expect(permission).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(permission).toBeHidden();
  await expect(reply).toHaveText(/Allowed, so I went ahead\./);

  // Esc denies, with the message written.
  await input.fill("permission again");
  await input.press("Enter");
  await permission.getByRole("textbox", { name: "Deny message" }).fill("not now");
  await page.keyboard.press("Escape");
  await expect(permission).toBeHidden();
  await expect(reply).toHaveText(/Denied: not now\. I stopped\./);

  // A question: a button per option, checkboxes for several, Enter sends.
  await input.fill("a question");
  await input.press("Enter");
  const question = chat.getByRole("region", { name: "Question" });
  await question.getByRole("button", { name: "Portuguese" }).click();
  await question.getByRole("checkbox", { name: "README.md" }).check();
  await question.getByRole("checkbox", { name: "CONTRIBUTING.md" }).check();
  await question.getByRole("checkbox", { name: "CONTRIBUTING.md" }).press("Enter");
  await expect(question).toBeHidden();
  await expect(reply).toHaveText(/You chose: Portuguese \/ README\.md, CONTRIBUTING\.md\./);

  // A plan: approving and accepting edits changes the mode the selector shows.
  const mode = chat.getByRole("combobox", { name: "Permission mode" });
  await expect(mode).toHaveText("Default");
  await input.fill("make a plan");
  await input.press("Enter");
  const plan = chat.getByRole("region", { name: "Plan approval" });
  await expect(plan.locator("pre")).toContainText("## Add CONTRIBUTING.md");
  await plan.getByRole("button", { name: "Approve and accept edits" }).click();
  await expect(plan).toBeHidden();
  await expect(mode).toHaveText("Accept edits");

  // The selector asks the service for another mode.
  await mode.click();
  await page.getByRole("option", { name: "Plan", exact: true }).click();
  await expect(mode).toHaveText("Plan");
});

test("chat: the header shows model and mode, Esc stops a turn, / lists the commands", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await page.getByRole("button", { name: "Start chat" }).click();
  const chat = page.getByRole("region", { name: "Chat" });
  const input = chat.getByRole("textbox", { name: "Message" });
  await expect(input).toBeEnabled();
  await expect(chat.locator(".chat-meta")).toHaveText("claude-mock · Default");

  // Esc in the message stops the running turn.
  await input.fill("list the files");
  await input.press("Enter");
  await expect(chat.getByRole("button", { name: "Stop" })).toBeVisible();
  await input.press("Escape");
  await expect(chat.locator('[data-role="note"]')).toHaveText("Interrupted");
  await expect(chat.getByRole("button", { name: "Send" })).toBeVisible();

  // `/` lists the commands; the arrows and Enter pick one.
  const commands = chat.getByRole("listbox", { name: "Commands" });
  await input.fill("/");
  await expect(commands.getByRole("option")).toHaveText(["/compact", "/clear", "/review"]);
  await input.press("c");
  await expect(commands.getByRole("option")).toHaveText(["/compact", "/clear"]);
  await input.press("ArrowDown");
  await expect(commands.getByRole("option", { selected: true })).toHaveText("/clear");
  await input.press("Enter");
  await expect(input).toHaveValue("/clear ");
  await expect(commands).toBeHidden();
  // Esc hides the list.
  await input.fill("/r");
  await expect(commands).toBeVisible();
  await input.press("Escape");
  await expect(commands).toBeHidden();
  await expect(input).toHaveValue("/r");
});

test("chat: a session opens as a chat with its history, and an ended chat resumes", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "main", exact: true }).first().click();
  // The side panel shows with a worktree.
  await page.getByRole("tab", { name: "Sessions" }).click();
  await page.getByRole("button", { name: "Actions for Checkout totals" }).click();
  await page.getByRole("menuitem", { name: "Open as Chat" }).click();
  await page.getByRole("button", { name: "Start chat" }).click();

  const chat = page.getByRole("region", { name: "Chat" });
  const entries = chat.locator(".transcript-entry");
  await expect(entries.first()).toHaveText("YouWhat are git worktrees?");
  await expect(entries).toHaveCount(2);

  // Once it ends, it resumes in its place, its history shown again.
  const input = chat.getByRole("textbox", { name: "Message" });
  await input.fill("crash");
  await input.press("Enter");
  await chat.getByRole("button", { name: "Resume" }).click();
  await expect(chat.getByRole("button", { name: "Resume" })).toBeHidden();
  await expect(entries).toHaveCount(2);
  await expect(
    page.getByRole("tablist", { name: "Open terminals and files" }).getByRole("tab"),
  ).toHaveCount(1);
});

test("chat: Claude's Markdown renders, and a wide table scrolls inside its message", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await expect(page.getByRole("dialog", { name: "Chat in this folder?" })).toBeVisible();
  await page.keyboard.press("Enter");
  const chat = page.getByRole("region", { name: "Chat" });
  const input = chat.getByRole("textbox", { name: "Message" });
  await input.fill("show markdown");
  await input.press("Enter");

  const message = chat.locator(".markdown", { has: page.locator("table") });
  await expect(message.getByRole("heading", { name: "Context Usage" })).toBeVisible();
  await expect(message.locator("strong")).toHaveText("Model:");
  await expect(message.locator("pre code")).toHaveText("bun test --coverage");
  await expect(message.locator("td").nth(1)).toHaveCSS("text-align", "right");
  // The table is wider than the chat: its box scrolls sideways, the conversation does not.
  const scroller = message.locator(".md-table");
  const sizes = await scroller.evaluate((el) => {
    const transcript = el.closest(".transcript") as HTMLElement;
    return {
      wide: el.scrollWidth > el.clientWidth,
      overflow: transcript.scrollWidth - transcript.clientWidth,
    };
  });
  expect(sizes).toEqual({ wide: true, overflow: 0 });
  await scroller.evaluate((el) => el.scrollBy(200, 0));
  await expect.poll(() => scroller.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
});
