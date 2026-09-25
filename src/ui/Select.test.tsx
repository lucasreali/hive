import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { Select, TYPE_AHEAD_MS } from "./Select";

afterEach(cleanup);

const OPTIONS = [
  { value: "a", label: "Apple" },
  { value: "b", label: "Banana" },
  { value: "c", label: "Blueberry" },
  { value: "d", label: "Cherry" },
];

/** A select that keeps its value, with a spy on changes; `onKey` sees what reaches the parent. */
function setup(value = "b", onKey = (_: React.KeyboardEvent) => {}) {
  const changed = mock((_: string) => {});
  function Host() {
    const [v, setV] = useState(value);
    return (
      // biome-ignore lint/a11y/noStaticElementInteractions: a stand-in for the dialog.
      <div onKeyDown={onKey}>
        <Select
          aria-label="Fruit"
          value={v}
          options={OPTIONS}
          onChange={(next) => {
            changed(next);
            setV(next);
          }}
        />
        <button type="button">Other</button>
      </div>
    );
  }
  render(<Host />);
  const trigger = screen.getByRole("combobox", { name: "Fruit" });
  return { trigger, changed };
}

const list = () => screen.queryByRole("listbox");
const active = (trigger: HTMLElement) =>
  document.getElementById(trigger.getAttribute("aria-activedescendant") ?? "")?.textContent;
const key = (trigger: HTMLElement, k: string, init = {}) =>
  fireEvent.keyDown(trigger, { key: k, ...init });

test("the trigger shows the value; a click opens the list under it, a pick closes it", () => {
  const { trigger, changed } = setup();
  expect(trigger.textContent).toBe("Banana");
  expect(trigger.getAttribute("aria-expanded")).toBe("false");
  expect(trigger.getAttribute("aria-activedescendant")).toBeNull();
  expect(list()).toBeNull();

  fireEvent.mouseDown(trigger, { button: 2 });
  expect(list()).toBeNull();
  fireEvent.mouseDown(trigger);
  expect(document.activeElement).toBe(trigger);
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
  expect(trigger.getAttribute("aria-controls")).toBe(list()?.id as string);
  const options = screen.getAllByRole("option");
  expect(options.map((o) => o.textContent)).toEqual(["Apple", "Banana", "Blueberry", "Cherry"]);
  expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual([
    "false",
    "true",
    "false",
    "false",
  ]);
  // The selected option carries the check mark; the active one starts on it.
  expect(options[1].querySelector(".select-check")).not.toBeNull();
  expect(active(trigger)).toBe("Banana");
  expect((list() as HTMLElement).style.top).toMatch(/px$/);

  // Hover moves the active option; clicking the list keeps the focus on the trigger.
  fireEvent.mouseMove(options[3]);
  expect(options[3].getAttribute("data-active")).toBe("true");
  expect(fireEvent.mouseDown(list() as HTMLElement)).toBe(false);
  fireEvent.click(options[3]);
  expect(changed).toHaveBeenCalledWith("d");
  expect(list()).toBeNull();
  expect(trigger.textContent).toBe("Cherry");
  expect(document.activeElement).toBe(trigger);

  // Picking the current value closes without a change; a second click on the trigger closes.
  fireEvent.mouseDown(trigger);
  fireEvent.click(screen.getByRole("option", { name: "Cherry" }));
  expect(changed).toHaveBeenCalledTimes(1);
  fireEvent.mouseDown(trigger);
  fireEvent.mouseDown(trigger);
  expect(list()).toBeNull();
});

test("the keyboard opens, moves and picks; Esc closes without reaching the dialog", () => {
  const escapes: string[] = [];
  const { trigger, changed } = setup("b", (e) => {
    if (e.key === "Escape") escapes.push(e.key);
  });
  trigger.focus();

  for (const k of ["ArrowDown", "ArrowUp", "Enter", " "]) {
    key(trigger, k);
    expect(active(trigger)).toBe("Banana");
    key(trigger, "Escape");
    expect(list()).toBeNull();
  }
  expect(escapes).toEqual([]);
  // Esc with the list closed is the dialog's.
  key(trigger, "Escape");
  expect(escapes).toEqual(["Escape"]);

  key(trigger, "ArrowDown");
  key(trigger, "ArrowDown");
  expect(active(trigger)).toBe("Blueberry");
  key(trigger, "End");
  key(trigger, "ArrowDown");
  expect(active(trigger)).toBe("Cherry");
  key(trigger, "Home");
  key(trigger, "ArrowUp");
  expect(active(trigger)).toBe("Apple");
  key(trigger, " ");
  expect(changed).toHaveBeenLastCalledWith("a");
  expect(list()).toBeNull();

  key(trigger, "End");
  expect(active(trigger)).toBe("Cherry");
  key(trigger, "Enter");
  expect(changed).toHaveBeenLastCalledWith("d");

  // Tab closes and lets the focus move on; other keys are left alone.
  key(trigger, "Home");
  expect(key(trigger, "Tab")).toBe(true);
  expect(list()).toBeNull();
  expect(key(trigger, "F2")).toBe(true);
  expect(key(trigger, "a", { ctrlKey: true })).toBe(true);
  expect(changed).toHaveBeenCalledTimes(2);
});

test("typing jumps to the option starting with the typed text", () => {
  const now = spyOn(Date, "now").mockReturnValue(10_000);
  try {
    const { trigger } = setup("a");
    key(trigger, "b");
    expect(active(trigger)).toBe("Banana");
    // A repeated first letter cycles among the options that start with it.
    now.mockReturnValue(10_000 + TYPE_AHEAD_MS);
    key(trigger, "b");
    expect(active(trigger)).toBe("Blueberry");
    now.mockReturnValue(20_000);
    key(trigger, "b");
    expect(active(trigger)).toBe("Banana");
    // Quick typing searches the whole text.
    key(trigger, "L");
    expect(active(trigger)).toBe("Blueberry");
    key(trigger, "x");
    expect(active(trigger)).toBe("Blueberry");
    now.mockReturnValue(30_000);
    key(trigger, "c");
    expect(active(trigger)).toBe("Cherry");
  } finally {
    now.mockRestore();
  }
});

test("blur, scrolling outside the list and resizing close it", () => {
  const { trigger } = setup();
  fireEvent.mouseDown(trigger);
  fireEvent.scroll(list() as HTMLElement);
  expect(list()).not.toBeNull();
  fireEvent.scroll(document);
  expect(list()).toBeNull();

  fireEvent.mouseDown(trigger);
  fireEvent(window, new Event("resize"));
  expect(list()).toBeNull();

  fireEvent.mouseDown(trigger);
  fireEvent.blur(trigger);
  expect(list()).toBeNull();
});

test("an unknown value shows the first option; disabled and a class pass through", () => {
  render(
    <Select
      aria-labelledby="l"
      className="compact"
      disabled
      value="zzz"
      options={OPTIONS}
      onChange={() => {}}
    />,
  );
  const trigger = screen.getByRole("combobox");
  expect(trigger.textContent).toBe("Apple");
  expect((trigger as HTMLButtonElement).disabled).toBe(true);
  expect(trigger.getAttribute("aria-labelledby")).toBe("l");
  expect(trigger.parentElement?.className).toBe("select compact");
});
