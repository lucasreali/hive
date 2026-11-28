import { expect, type Page, test } from "@playwright/test";

// 29.7: what is typed into a terminal reaches its PTY once (the mock records each write): plain
// text, dead-key accents as macOS' WebKit sends them (an accent the next key does not combine
// with, the apostrophe of "don't", was sent twice and its key lost), an IME composition; again
// after the host's link drops and the terminal is attached again, and in a split. The dead keys'
// handling is macOS' (WebKit's events there), so the pages say Mac on every host, and Hive's
// command key is Cmd.
test.use({
  userAgent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
});

/** What terminal `id` was sent, every write joined, as the mock recorded it. */
function sent(page: Page, id: number): Promise<string> {
  return page.evaluate(async (id) => {
    const url = "/src/transport/mock.ts";
    const { MOCK_INPUT } = await import(/* @vite-ignore */ url);
    return (MOCK_INPUT.get(id) ?? []).join("");
  }, id);
}

/** The id of the terminal that has the keyboard. */
async function focused(page: Page): Promise<number> {
  const id = await page.evaluate(
    () => document.activeElement?.closest<HTMLElement>(".terminal-pane")?.dataset.terminal,
  );
  return Number(id);
}

/**
 * A dead key then `next`, as a WKWebView sends them with a Brazilian layout (traced in one):
 * the accent's composition is committed before the next key's keydown, which is the composing
 * one (229) when the two combine (´a → á), else the dead key's own code (`code`) with both
 * characters as its key ("'t"), followed by the next key's keypress and text unless taken.
 * Each native event in its own task, as WebKit sends them.
 */
function deadKey(page: Page, accent: string, code: number, next: string, combined = "") {
  return page.evaluate(
    async ({ accent, code, next, combined }) => {
      const area = document.activeElement as HTMLTextAreaElement;
      const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
      const composition = (type: string, data: string) =>
        area.dispatchEvent(new CompositionEvent(type, { data, bubbles: true }));
      const input = (inputType: string, data: string | null) =>
        area.dispatchEvent(
          new InputEvent("input", { inputType, data, bubbles: true, composed: true }),
        );
      const key = (type: string, key: string, keyCode: number, extra: object = {}) => {
        const event = new KeyboardEvent(type, { key, bubbles: true, cancelable: true });
        for (const [name, value] of Object.entries({ keyCode, which: keyCode, ...extra })) {
          Object.defineProperty(event, name, { value });
        }
        area.dispatchEvent(event);
        return event;
      };
      const before = area.value;
      composition("compositionstart", "");
      composition("compositionupdate", accent);
      area.value = before + accent;
      input("insertCompositionText", accent);
      key("keydown", "Dead", 229, { isComposing: true });
      await tick();
      key("keyup", accent, code, { isComposing: true });
      await tick();
      area.value = before;
      input("deleteCompositionText", null);
      area.value = before + (combined || accent);
      input("insertFromComposition", combined || accent);
      composition("compositionend", combined || accent);
      if (combined) key("keydown", combined, 229);
      else if (!key("keydown", accent + next, code).defaultPrevented) {
        const charCode = accent.charCodeAt(0);
        if (!key("keypress", accent + next, charCode, { charCode }).defaultPrevented) {
          area.value += next;
          input("insertText", next);
        }
      }
      await tick();
      key("keyup", next, next.toUpperCase().charCodeAt(0));
      await tick();
    },
    { accent, code, next, combined },
  );
}

/** An IME's composition (Japanese): its text grows, then it is committed as a whole. */
function compose(page: Page, steps: string[]) {
  return page.evaluate((steps) => {
    const area = document.activeElement as HTMLTextAreaElement;
    const before = area.value;
    const event = (type: string, data: string) =>
      area.dispatchEvent(new CompositionEvent(type, { data, bubbles: true }));
    event("compositionstart", "");
    for (const step of steps) {
      area.value = before + step;
      event("compositionupdate", step);
    }
    event("compositionend", steps.at(-1) ?? "");
  }, steps);
}

/** Types plain text, accents (combined and not) and a composition into the focused terminal. */
async function typeAll(page: Page) {
  await page.keyboard.type("hi ");
  await deadKey(page, "'", 222, "a", "á");
  await deadKey(page, "˜", 192, "a", "ã");
  await page.keyboard.type(" don");
  await deadKey(page, "'", 222, "t");
  await deadKey(page, '"', 222, "x");
  await compose(page, ["に", "にほ", "日本"]);
  await expect
    .poll(async () => page.evaluate(() => document.activeElement?.tagName))
    .toBe("TEXTAREA");
}

const TYPED = `hi áã don't"x日本`;

test("each key typed reaches the PTY once: text, accents, a composition; after a reattach and in a split", async ({
  page,
}) => {
  await page.goto("/?mock=remote");
  await expect(page.locator(".hosts-item")).toHaveText("1 host");
  await page.getByRole("combobox", { name: "Space" }).click();
  await page.getByRole("option", { name: "Default · gpu-box", exact: true }).click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect(page.locator(".terminal-pane:not([hidden]) .xterm-helper-textarea")).toBeFocused();
  const first = await focused(page);

  await typeAll(page);
  await expect.poll(() => sent(page, first)).toBe(TYPED);

  // The host's link drops and comes back: the terminal is attached again and redrawn.
  await page.keyboard.type("\r");
  await page.keyboard.type("drop-link\r");
  const item = page.locator(".hosts-item");
  await expect(item).toHaveAttribute("data-status", "disconnected");
  await item.click();
  await page
    .getByRole("dialog", { name: "Hosts" })
    .getByRole("button", { name: "Reconnect gpu-box" })
    .click();
  await expect(item).toHaveAttribute("data-status", "connected");
  await page.keyboard.press("Escape");
  await page.locator(".terminal-pane:not([hidden]) .xterm-screen").click();
  const before = await sent(page, first);
  await typeAll(page);
  await expect.poll(() => sent(page, first)).toBe(before + TYPED);

  // Split beside a new terminal, which takes the keys; the first gets nothing more.
  await page.keyboard.press("Meta+Shift+D");
  await expect(page.locator(".terminal-pane:not([hidden])")).toHaveCount(2);
  const second = await focused(page);
  expect(second).not.toBe(first);
  await typeAll(page);
  await expect.poll(() => sent(page, second)).toBe(TYPED);
  expect(await sent(page, first)).toBe(before + TYPED);
});
