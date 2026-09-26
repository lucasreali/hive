import { afterEach, beforeAll, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";
import { type ChatEntry, mergeEntries } from "../store";
import { CHAT_LABELS, ConversationView, type Labels, ordered } from "./ConversationView";

beforeAll(() => {
  // happy-dom has no layout: give the list its CSS size so the virtualizer shows entries.
  for (const [key, size] of [
    ["offsetHeight", 2000],
    ["offsetWidth", 600],
  ] as const) {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, key)?.get;
    Object.defineProperty(HTMLElement.prototype, key, {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains("transcript") ? size : original?.call(this);
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
