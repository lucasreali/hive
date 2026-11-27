import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { placeList, Select, TYPE_AHEAD_MS } from "./Select";

afterEach(cleanup);

test("a disabled option shows, with its tooltip, but is never picked", () => {
  const changed = mock((_: string) => {});
  const options = [
    { value: "a", label: "Apple" },
    { value: "b", label: "Banana", disabled: true, title: "Out of season" },
  ];
  render(<Select aria-label="Fruit" value="a" options={options} onChange={changed} />);
  const trigger = screen.getByRole("combobox", { name: "Fruit" });
  fireEvent.keyDown(trigger, { key: "ArrowDown" });
  const banana = screen.getByRole("option", { name: "Banana" });
  expect([banana.getAttribute("aria-disabled"), banana.title]).toEqual(["true", "Out of season"]);
  expect(screen.getByRole("option", { name: "Apple" }).getAttribute("aria-disabled")).toBeNull();
  fireEvent.keyDown(trigger, { key: "ArrowDown" });
  fireEvent.keyDown(trigger, { key: "Enter" });
  fireEvent.click(banana);
  expect(changed).not.toHaveBeenCalled();
  expect(screen.getByRole("listbox")).toBeDefined();
});

const OPTIONS = [
  { value: "a", label: "Apple" },
  { value: "b", label: "Banana" },
  { value: "c", label: "Blueberry" },
  { value: "d", label: "Cherry" },
];

/** A select that keeps its value, reporting changes to `changed`; `onKey` sees what reaches the parent. */
function Host({
  value,
  onKey,
  changed,
}: {
  value: string;
  onKey: (event: React.KeyboardEvent) => void;
  changed: (next: string) => void;
}) {
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

/** A select that keeps its value, with a spy on changes; `onKey` sees what reaches the parent. */
function setup(value = "b", onKey = (_: React.KeyboardEvent) => {}) {
  const changed = mock((_: string) => {});
  render(<Host value={value} onKey={onKey} changed={changed} />);
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

test("near the bottom of the window the list opens above the trigger", () => {
  const { trigger } = setup();
  const bottom = window.innerHeight - 10;
  spyOn(trigger, "getBoundingClientRect").mockReturnValue(
    DOMRect.fromRect({ x: 0, y: bottom - 30, width: 120, height: 30 }),
  );
  fireEvent.mouseDown(trigger);
  const style = (list() as HTMLElement).style;
  expect([style.top, style.bottom, style.maxHeight]).toEqual(["", "42px", "240px"]);
  mock.restore();
});

// 28.17: the list is placed where it fits, never outside the window.
const VIEW = { width: 1000, height: 700 };
const rect = (left: number, top: number, width = 120, height = 30) => ({
  left,
  top,
  right: left + width,
  bottom: top + height,
});

test("a list that fits opens under the trigger, left-aligned to it", () => {
  expect(placeList(rect(100, 50), 200, VIEW)).toEqual({ left: 100, top: 82, maxHeight: 240 });
  // Exactly up to the window's margin still fits.
  expect(placeList(rect(792, 50), 200, VIEW).left).toBe(792);
});

test("a list that would pass the right edge is right-aligned to the trigger", () => {
  // The trigger ends at 990; a 300 px list from 870 would end at 1170.
  expect(placeList(rect(870, 50), 300, VIEW)).toEqual({ left: 690, top: 82, maxHeight: 240 });
  // A trigger at the very edge: the list keeps the window's margin.
  expect(placeList(rect(960, 50, 40), 300, VIEW).left).toBe(692);
  // Wider than the trigger's right allows on the left: it keeps the left margin.
  expect(placeList(rect(10, 50, 40), 990, VIEW).left).toBe(8);
});

test("a list that would pass the bottom edge opens above the trigger", () => {
  expect(placeList(rect(100, 640), 200, VIEW)).toEqual({ left: 100, bottom: 62, maxHeight: 240 });
  // With little room either way it takes the larger side, scrolling.
  expect(placeList(rect(100, 100, 120, 30), 200, { width: 1000, height: 300 })).toEqual({
    left: 100,
    top: 132,
    maxHeight: 162,
  });
  expect(placeList(rect(100, 170, 120, 30), 200, { width: 1000, height: 300 })).toEqual({
    left: 100,
    bottom: 132,
    maxHeight: 162,
  });
  // Under the minimum room below it still opens there when above is smaller.
  expect(placeList(rect(100, 20, 120, 30), 200, { width: 1000, height: 90 }).maxHeight).toBe(80);
});

test("the list in the DOM gets the placed position", () => {
  const { trigger } = setup();
  spyOn(trigger, "getBoundingClientRect").mockReturnValue(
    DOMRect.fromRect({ x: window.innerWidth - 50, y: 40, width: 40, height: 30 }),
  );
  fireEvent.mouseDown(trigger);
  const style = (list() as HTMLElement).style;
  // happy-dom lays nothing out, so the list measures 0 px wide.
  expect([style.left, style.top, style.minWidth, style.maxWidth]).toEqual([
    `${window.innerWidth - 50}px`,
    "72px",
    "40px",
    `${window.innerWidth - 16}px`,
  ]);
  mock.restore();
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

// 28.18: labelled groups; the labels are not options, and the same value may be in two groups.
test("groups show under their labels; keys and type-ahead skip the labels", () => {
  const changed = mock((_: string) => {});
  const options = [
    { value: "none", label: "None" },
    { value: "all", label: "All" },
    {
      group: "Work",
      options: [
        { value: "hive", label: "hive" },
        { value: "one", label: "one" },
      ],
    },
    {
      group: "Home",
      options: [
        { value: "hive", label: "hive" },
        { value: "garden", label: "garden" },
      ],
    },
    { value: "other", label: "Other" },
  ];
  render(<Select aria-label="Project" value="hive" options={options} onChange={changed} />);
  const trigger = screen.getByRole("combobox", { name: "Project" });
  expect(trigger.textContent).toBe("hive");
  key(trigger, "ArrowDown");
  const groups = screen.getAllByRole("group");
  expect(groups.map((g) => g.getAttribute("aria-labelledby"))).toEqual(
    groups.map((g) => g.firstElementChild?.id ?? null),
  );
  expect(groups.map((g) => g.firstElementChild?.textContent)).toEqual(["Work", "Home"]);
  expect(groups.map((g) => screen.getByRole("listbox").contains(g))).toEqual([true, true]);
  const shown = screen.getAllByRole("option");
  expect(shown.map((o) => o.textContent)).toEqual([
    "None",
    "All",
    "hive",
    "one",
    "hive",
    "garden",
    "Other",
  ]);
  // The value is selected in both groups; the active option starts on its first.
  expect(shown.map((o) => o.getAttribute("aria-selected"))).toEqual([
    "false",
    "false",
    "true",
    "false",
    "true",
    "false",
    "false",
  ]);
  expect(shown.filter((o) => o.querySelector(".select-check")).length).toBe(2);
  expect(active(trigger)).toBe("hive");
  // ↓ from Work's last option goes to Home's first, past its label; ↑ comes back.
  key(trigger, "ArrowDown");
  key(trigger, "ArrowDown");
  expect(document.getElementById(trigger.getAttribute("aria-activedescendant") ?? "")).toBe(
    shown[4],
  );
  key(trigger, "ArrowUp");
  expect(active(trigger)).toBe("one");
  // Type-ahead finds options only, never a label ("Work", "Home").
  const now = spyOn(Date, "now").mockReturnValue(10_000);
  key(trigger, "h");
  expect(document.getElementById(trigger.getAttribute("aria-activedescendant") ?? "")).toBe(
    shown[4],
  );
  now.mockReturnValue(20_000);
  key(trigger, "w");
  expect(document.getElementById(trigger.getAttribute("aria-activedescendant") ?? "")).toBe(
    shown[4],
  );
  // Picking the same project in the other group changes nothing; another option does.
  key(trigger, "Enter");
  expect(changed).not.toHaveBeenCalled();
  fireEvent.mouseDown(trigger);
  fireEvent.click(screen.getByRole("option", { name: "garden" }));
  expect(changed).toHaveBeenCalledWith("garden");
  now.mockRestore();
});
