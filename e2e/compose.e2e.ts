import { expect, type Page, test } from "@playwright/test";
import { mod } from "./host";

// Accents typed with dead keys (´ ~ ^ on a pt-BR layout) and ç (17.14): they reach the terminal
// and the palette as typed, and the Enter that ends a composition is the composition's.

/** The text of terminal 1's buffer, read from the page's own terminal manager. */
function screen(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const buffer = terminal(1).buffer.active;
    const lines: string[] = [];
    for (let y = 0; y < buffer.length; y++) lines.push(buffer.getLine(y).translateToString(true));
    return lines.join("\n").trimEnd();
  });
}

/**
 * A dead key then a letter, as the engine reports them to the focused field: a composition
 * that starts with the accent and ends with the accented letter, which the field then holds.
 */
function compose(page: Page, accent: string, letter: string): Promise<void> {
  return page.evaluate(
    ([accent, letter]) => {
      const field = document.activeElement as HTMLInputElement | HTMLTextAreaElement;
      const event = (type: string, data: string) =>
        field.dispatchEvent(new CompositionEvent(type, { data, bubbles: true }));
      event("compositionstart", "");
      event("compositionupdate", accent);
      field.value += letter;
      event("compositionupdate", letter);
      event("compositionend", letter);
      field.dispatchEvent(new InputEvent("input", { data: letter, bubbles: true }));
    },
    [accent, letter],
  );
}

test("dead keys and ç: the terminal and the palette get the letters as typed", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Projects" })
    .getByRole("button", { name: "fix-login" })
    .click();
  await page.getByTitle("New terminal or file").click();
  await page.getByRole("menuitem", { name: "Terminal" }).click();
  await expect.poll(() => screen(page)).toBe("mock$");

  // ´a and ~a composed, ç typed straight (its own key on a pt-BR layout). xterm sends a
  // composition once it ended, on a later task: each is awaited before the next key.
  await compose(page, "´", "á");
  await expect.poll(() => screen(page)).toBe("mock$ á");
  await compose(page, "~", "ã");
  await expect.poll(() => screen(page)).toBe("mock$ áã");
  await page.keyboard.type("ç");
  await expect.poll(() => screen(page)).toBe("mock$ áãç");

  await page.keyboard.press(`${mod}+Shift+P`);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const field = palette.getByRole("textbox");
  await expect(field).toBeFocused();
  await compose(page, "´", "á");
  await compose(page, "~", "ã");
  await page.keyboard.type("ç");
  await expect(field).toHaveValue("áãç");

  // An Enter that ends a composition runs nothing; the next Enter runs the row.
  await field.fill("open settings");
  await field.dispatchEvent("keydown", { key: "Enter", isComposing: true });
  await expect(palette).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(palette).toBeHidden();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
});

test.describe("Option as Meta on one side (D20.1)", () => {
  // macOS' WebKit on every host: the Option keys are Meta only on macOS.
  test.use({
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
  });

  test("with the left Option as Meta, left ⌥P sends Esc P and right ⌥C types ç", async ({
    page,
    browserName,
  }) => {
    test.skip(browserName !== "webkit", "WebKit is macOS' engine");
    // xterm tells macOS by the platform: off a Mac its Option would send Esc + the key itself.
    await page.addInitScript(() =>
      Object.defineProperty(navigator, "platform", { get: () => "MacIntel" }),
    );
    await page.goto("/");
    await page
      .getByRole("navigation", { name: "Projects" })
      .getByRole("button", { name: "fix-login" })
      .click();
    await page.getByTitle("New terminal or file").click();
    await page.getByRole("menuitem", { name: "Terminal" }).click();
    await expect.poll(() => screen(page)).toBe("mock$");
    // What terminal 1 sends, and whether each key's keydown was taken (macOS then types nothing).
    await page.evaluate(async () => {
      const w = window as unknown as { sent: string[]; taken: string[] };
      w.sent = [];
      w.taken = [];
      const [terminals, store] = ["/src/terminals.ts", "/src/store.ts"];
      const [{ terminal }, { useHive }] = await Promise.all([
        import(/* @vite-ignore */ terminals),
        import(/* @vite-ignore */ store),
      ]);
      terminal(1).onData((data: string) => w.sent.push(data));
      window.addEventListener("keydown", (e) => {
        if (e.code.startsWith("Key")) w.taken.push(`${e.code}:${e.defaultPrevented}`);
      });
      const { settings } = useHive.getState();
      useHive.setState({
        settings: { ...settings, terminal: { ...settings.terminal, option_as_meta: "left" } },
      });
    });
    const read = () =>
      page.evaluate(() => {
        const { sent, taken } = window as unknown as { sent: string[]; taken: string[] };
        return { sent, taken };
      });

    await page.keyboard.down("AltLeft");
    await page.keyboard.press("KeyP");
    await page.keyboard.up("AltLeft");
    await page.keyboard.down("AltRight");
    await page.keyboard.press("KeyC");
    // Playwright types no text under Option: this is the ç macOS composes for a keydown left alone.
    if (!(await read()).taken.includes("KeyC:true")) await page.keyboard.insertText("ç");
    await page.keyboard.up("AltRight");
    await expect.poll(() => screen(page)).toBe("mock$ ç");
    expect(await read()).toEqual({ sent: ["\x1bp", "ç"], taken: ["KeyP:true", "KeyC:false"] });
  });
});
