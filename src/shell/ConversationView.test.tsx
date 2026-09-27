import { afterEach, beforeAll, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import type { TranscriptEntry } from "../store";
import { ConversationView } from "./ConversationView";

beforeAll(() => {
  // happy-dom has no layout: give the list its CSS size so the virtualizer shows entries, and each
  // entry a height so a scrolled list has a top row.
  for (const [key, size] of [
    ["offsetHeight", 2000],
    ["offsetWidth", 600],
    ["clientHeight", 2000],
    ["scrollHeight", 6000],
  ] as const) {
    const original =
      Object.getOwnPropertyDescriptor(HTMLElement.prototype, key)?.get ??
      Object.getOwnPropertyDescriptor(Element.prototype, key)?.get;
    Object.defineProperty(HTMLElement.prototype, key, {
      configurable: true,
      get(this: HTMLElement) {
        if (this.classList.contains("transcript")) return size;
        if (key === "offsetHeight" && this.classList.contains("transcript-entry")) return 100;
        return original?.call(this);
      },
    });
  }
});

afterEach(cleanup);

const entry = (role: TranscriptEntry["role"], text: string, tool: string | null = null) =>
  ({ role, text, tool }) satisfies TranscriptEntry;

test("each role has its row: the prompt, the subagent's Markdown, tools by name", () => {
  const list = [
    entry("user", "**raw** <b>x</b>"),
    entry("assistant", "**bold** text"),
    entry("tool", "ls -la", "Bash"),
  ];
  const view = render(
    <ConversationView entries={list}>
      <div className="hint">hint</div>
    </ConversationView>,
  );
  const rows = [...view.container.querySelectorAll(".transcript-entry")];
  expect(
    rows.map((row) => [
      row.getAttribute("data-role"),
      row.querySelector(".transcript-role")?.textContent,
      row.querySelector(".transcript-text")?.textContent,
    ]),
  ).toEqual([
    ["user", "Prompt", "**raw** <b>x</b>"],
    ["assistant", "Subagent", "bold text"],
    ["tool", "Bash", "ls -la"],
  ]);
  expect(view.getByText("hint")).toBeDefined();
  const [prompt, reply] = rows.map((row) => row.querySelector(".transcript-text"));
  expect(prompt?.querySelector("strong, b, .markdown")).toBeNull();
  expect(reply?.querySelector(".markdown strong")?.textContent).toBe("bold");
});

/** 30 turns of a prompt and its reply, 100 px each (rows 2k and 2k + 1); row 11 is a tool's. */
const turns = (count = 30) =>
  Array.from({ length: count }, (_, k) => [
    entry("user", `Prompt ${k}\nsecond line`),
    k === 5 ? entry("tool", "a tool call", "Read") : entry("assistant", `Reply ${k}`),
  ]).flat();

/** The scroller (6000 px of content in a 2000 px view) and a spy for the virtualizer's scrolls. */
function scroller(container: HTMLElement) {
  const el = container.querySelector(".transcript") as HTMLElement;
  const scrollTo = mock((_: ScrollToOptions) => {});
  el.scrollTo = scrollTo as unknown as typeof el.scrollTo;
  const scroll = (top: number) => {
    el.scrollTop = top;
    fireEvent.scroll(el);
  };
  return { scrollTo, scroll };
}

test("scrolled up, a button and the prompt of the view's top show; at the bottom they go", () => {
  const view = render(<ConversationView entries={turns()} />);
  const { scroll } = scroller(view.container);
  expect(view.queryByTitle("Scroll to the bottom")).toBeNull();
  expect(view.container.querySelector(".conversation-prompt")).toBeNull();

  scroll(1050); // the top row is prompt 5
  expect(view.getByTitle("Scroll to the bottom")).toBeDefined();
  const bar = view.getByTitle("Scroll to this message");
  expect(bar.querySelector(".transcript-role")?.textContent).toBe("Prompt");
  // CSS keeps it to one line.
  expect(bar.querySelector(".conversation-prompt-text")?.textContent).toBe("Prompt 5\nsecond line");

  scroll(1150); // a tool call is not a prompt
  expect(bar.textContent).toContain("Prompt 5");
  scroll(1250);
  expect(bar.textContent).toContain("Prompt 6");

  scroll(3990); // within a line of the bottom (6000 - 2000)
  expect(view.queryByTitle("Scroll to the bottom")).toBeNull();
  expect(view.queryByTitle("Scroll to this message")).toBeNull();
});

test("new entries keep a scrolled-up view in place, and follow at the bottom", () => {
  const list = turns();
  const view = render(<ConversationView entries={list} />);
  const { scrollTo, scroll } = scroller(view.container);
  scroll(1050);
  scrollTo.mockClear();
  const more = [...list, entry("assistant", "More")];
  view.rerender(<ConversationView entries={more} />);
  expect(scrollTo).not.toHaveBeenCalled();

  scroll(4000);
  view.rerender(<ConversationView entries={[...more, entry("assistant", "Again")]} />);
  expect(scrollTo).toHaveBeenCalled();
});

test("the button scrolls smoothly to the newest entry, the bar to its prompt", () => {
  const view = render(<ConversationView entries={turns()} />);
  const { scrollTo, scroll } = scroller(view.container);
  scroll(1250);
  scrollTo.mockClear();
  fireEvent.click(view.getByTitle("Scroll to this message"));
  // Prompt 6 starts at 1200, shown below the bar.
  expect(scrollTo.mock.calls[0]?.[0]).toMatchObject({ top: 1200 - 32 });

  scrollTo.mockClear();
  fireEvent.click(view.getByTitle("Scroll to the bottom"));
  expect(scrollTo.mock.calls[0]?.[0]).toMatchObject({ top: 4000, behavior: "smooth" });
});
