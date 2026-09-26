import { afterEach, beforeAll, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { type ChatEntry, mergeEntries } from "../store";
import { CHAT_LABELS, ConversationView, imageUrl, type Labels, ordered } from "./ConversationView";

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

const entry = (id: number, kind: ChatEntry["kind"], text: string, more: Partial<ChatEntry> = {}) =>
  ({
    id,
    kind,
    text,
    tool: null,
    parent: null,
    status: null,
    output: null,
    image: null,
    ...more,
  }) satisfies ChatEntry;

test("a subagent's entries show together, where the first one arrived", () => {
  const list = [
    entry(1, "tool", "a", { tool: "Agent" }),
    entry(2, "user", "a1", { parent: "A" }),
    entry(3, "tool", "b", { tool: "Agent" }),
    entry(4, "user", "b1", { parent: "B" }),
    entry(5, "assistant", "a2", { parent: "A" }),
    entry(6, "assistant", "done"),
  ];
  expect(ordered(list).map((e) => e.id)).toEqual([1, 2, 5, 3, 4, 6]);
});

test("each kind of entry has its row: text, collapsed thinking, tools with status and output", () => {
  const list = [
    entry(1, "user", "Hello"),
    entry(2, "thinking", "Let me look."),
    entry(3, "assistant", "Hi."),
    entry(4, "tool", "ls -la", { tool: "Bash", status: "running" }),
    entry(5, "tool", "notes.txt", { tool: "Read", status: "ok", output: "alpha\n" }),
    entry(6, "tool", "rm x", { tool: "Bash", status: "error", output: "denied" }),
    entry(7, "user", "Count.", { parent: "A" }),
    entry(8, "assistant", "Two.", { parent: "A" }),
    entry(9, "error", "API Error: 529"),
    entry(10, "note", "Interrupted"),
    entry(11, "divider", "Conversation compacted"),
    entry(12, "usage", "2.3 s"),
  ];
  const view = render(
    <ConversationView entries={list} labels={CHAT_LABELS}>
      <div className="hint">hint</div>
    </ConversationView>,
  );
  const rows = [...view.container.querySelectorAll(".transcript-entry")];
  expect(
    rows.map((row) => [
      row.getAttribute("data-role"),
      row.getAttribute("data-status"),
      row.hasAttribute("data-nested"),
      row.querySelector(".transcript-role")?.textContent ?? null,
      row.querySelector(".transcript-text")?.textContent,
    ]),
  ).toEqual([
    ["user", null, false, "You", "Hello"],
    ["thinking", null, false, null, "ThinkingLet me look."],
    ["assistant", null, false, "Claude", "Hi."],
    ["tool", "running", false, "Bash", "ls -la"],
    ["tool", "ok", false, "Read", "notes.txt"],
    ["tool", "error", false, "Bash", "rm x"],
    ["user", null, true, "Prompt", "Count."],
    ["assistant", null, true, "Subagent", "Two."],
    ["error", null, false, "Error", "API Error: 529"],
    ["note", null, false, null, "Interrupted"],
    ["divider", null, false, null, "Conversation compacted"],
    ["usage", null, false, null, "2.3 s"],
  ]);
  expect(view.getByText("hint")).toBeDefined();
  // Thinking and a finished tool's output start collapsed; a running tool has no output.
  const details = (i: number) => rows[i]?.querySelector("details") ?? null;
  expect([details(1)?.open, details(3), details(4)?.open]).toEqual([false, null, false]);
  expect(details(4)?.querySelector(".tool-output")?.textContent).toBe("alpha\n");
  expect(
    rows.slice(3, 6).map((r) => r.querySelector(".tool-status")?.getAttribute("aria-label")),
  ).toEqual(["running", "done", "failed"]);
  // A tool entry without a status (6.10's transcript) shows no icon.
  view.rerender(
    <ConversationView entries={[entry(1, "tool", "x", { tool: "Grep" })]} labels={CHAT_LABELS} />,
  );
  expect(view.container.querySelector(".tool-status")).toBeNull();
});

test("live text renders its own row again, not the others", () => {
  let reads = 0;
  const labels = new Proxy(CHAT_LABELS, {
    get: (target, key: keyof Labels) => {
      reads += 1;
      return target[key];
    },
  });
  const [a, b] = [entry(1, "user", "Hi"), entry(2, "assistant", "The te")];
  const view = render(<ConversationView entries={[a, b]} labels={labels} />);
  const before = reads;
  const grown = mergeEntries([a, b], [{ ...b, text: "The text" }], true);
  view.rerender(<ConversationView entries={grown} labels={labels} />);
  expect(reads - before).toBe(1);
  const texts = [...view.container.querySelectorAll(".transcript-text")];
  expect(texts.map((t) => t.textContent)).toEqual(["Hi", "The text"]);
});

test("images show from data: URLs, in messages and tool results, and grow when clicked", () => {
  const image = { media_type: "image/png", data: "iVBORw0KGgo=" };
  const gif = { media_type: "image/gif", data: "R0lGODlh" };
  const list = [
    entry(1, "user", "Look", { image }),
    entry(2, "tool", "/p/a.gif", { tool: "Read", status: "ok", output: "", image: gif }),
    entry(3, "assistant", "A dot."),
  ];
  const view = render(<ConversationView entries={list} labels={CHAT_LABELS} />);
  const sources = [...view.container.querySelectorAll("img")].map((img) => img.src);
  expect(sources).toEqual(["data:image/png;base64,iVBORw0KGgo=", "data:image/gif;base64,R0lGODlh"]);
  const [button] = view.getAllByTitle("Enlarge the image") as [HTMLElement];
  expect(button.textContent).toBe("");
  fireEvent.click(button);
  expect(button.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(view.getByTitle("Shrink the image"));
  expect(button.getAttribute("aria-pressed")).toBe("false");
  expect(imageUrl(gif)).toBe("data:image/gif;base64,R0lGODlh");
});

/** 30 turns of a prompt and its reply, 100 px each (rows 2k and 2k + 1); row 11 is a subagent's. */
const turns = (count = 30) =>
  Array.from({ length: count }, (_, k) => [
    entry(2 * k, "user", `Prompt ${k}\nsecond line`, {
      image: { media_type: "image/png", data: "iVBORw0KGgo=" },
    }),
    k === 5
      ? entry(2 * k + 1, "user", "a subagent's prompt", { parent: "A" })
      : entry(2 * k + 1, "assistant", `Reply ${k}`),
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
  const onScroll = mock((_offset: number, _atBottom: boolean) => {});
  const view = render(<ConversationView entries={turns()} labels={CHAT_LABELS} onScroll={onScroll} />);
  const { scroll } = scroller(view.container);
  expect(view.queryByTitle("Scroll to the bottom")).toBeNull();
  expect(view.container.querySelector(".conversation-prompt")).toBeNull();

  scroll(1050); // the top row is prompt 5
  expect(view.getByTitle("Scroll to the bottom")).toBeDefined();
  const bar = view.getByTitle("Scroll to this message");
  expect(bar.querySelector(".transcript-role")?.textContent).toBe("You");
  // Text only: no image, and CSS keeps it to one line.
  expect(bar.querySelector(".conversation-prompt-text")?.textContent).toBe(
    "Prompt 5\nsecond line",
  );
  expect(bar.querySelector("img")).toBeNull();
  expect(onScroll).toHaveBeenLastCalledWith(1050, false);

  scroll(1150); // a subagent's prompt is not the user's
  expect(bar.textContent).toContain("Prompt 5");
  scroll(1250);
  expect(bar.textContent).toContain("Prompt 6");

  scroll(3990); // within a line of the bottom (6000 - 2000)
  expect(view.queryByTitle("Scroll to the bottom")).toBeNull();
  expect(view.queryByTitle("Scroll to this message")).toBeNull();
  expect(onScroll).toHaveBeenLastCalledWith(3990, true);
});

test("new entries keep a scrolled-up view in place, and follow at the bottom", () => {
  const list = turns();
  const view = render(<ConversationView entries={list} labels={CHAT_LABELS} />);
  const { scrollTo, scroll } = scroller(view.container);
  scroll(1050);
  scrollTo.mockClear();
  const more = [...list, entry(100, "assistant", "More")];
  view.rerender(<ConversationView entries={more} labels={CHAT_LABELS} />);
  expect(scrollTo).not.toHaveBeenCalled();

  scroll(4000);
  view.rerender(
    <ConversationView entries={[...more, entry(101, "assistant", "Again")]} labels={CHAT_LABELS} />,
  );
  expect(scrollTo).toHaveBeenCalled();
});

test("the button scrolls smoothly to the newest entry, the bar to its prompt", () => {
  const view = render(<ConversationView entries={turns()} labels={CHAT_LABELS} />);
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

test("a view can start at a saved offset instead of the bottom", () => {
  const scrollTo = mock((_: ScrollToOptions) => {});
  const original = HTMLElement.prototype.scrollTo;
  HTMLElement.prototype.scrollTo = scrollTo as unknown as typeof original;
  try {
    const view = render(
      <ConversationView entries={turns()} labels={CHAT_LABELS} initialOffset={1050} />,
    );
    // Then the virtualizer keeps that content in place as rows above it are measured.
    expect(scrollTo.mock.calls[0]?.[0].top).toBe(1050);
    expect(view.getByTitle("Scroll to the bottom")).toBeDefined();
  } finally {
    HTMLElement.prototype.scrollTo = original;
  }
});
