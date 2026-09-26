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

test("chat: permission, question and plan cards pin above the composer and answer by keyboard", async ({
  page,
}) => {
  await page.goto("/");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await tree.getByRole("button", { name: "fix-login" }).click();
  await page.getByTitle("New terminal, agent or file").click();
  await page.getByRole("menuitem", { name: "Agent" }).click();
  await page.getByRole("button", { name: "Start chat Enter" }).click();
  const chat = page.getByRole("region", { name: "Chat" });
  const input = chat.getByRole("textbox", { name: "Message" });
  await expect(input).toBeEnabled();
  const reply = chat.locator('[data-role="assistant"]').last();

  // A permission takes the focus; Enter allows it and the card goes.
  await input.fill("ask permission to clean");
  await input.press("Enter");
  const permission = chat.getByRole("region", { name: "Permission request" });
  await expect(permission).toBeFocused();
  await expect(permission.locator("pre")).toHaveText("rm -rf target");
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
  await page.getByRole("button", { name: "Start chat Enter" }).click();
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
