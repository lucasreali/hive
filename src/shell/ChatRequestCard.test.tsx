import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ChatRequest } from "../store";
import { transport } from "../transport";
import { MOCK_CHAT_REQUESTS } from "../transport/mockChat";
import { ChatRequestCard } from "./ChatRequestCard";

afterEach(() => {
  mock.restore();
  cleanup();
});

function card(kind: ChatRequest["kind"]) {
  mock.restore();
  const answer = spyOn(transport, "chatAnswer").mockResolvedValue();
  const request = { id: "req_1", ...MOCK_CHAT_REQUESTS[kind] };
  render(<ChatRequestCard chat={4} request={request} />);
  const region = screen.getByRole("region");
  const answers = () => answer.mock.calls.map(([chat, id, value]) => [chat, id, value]);
  return { region, answers };
}

const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const text = (name: string) => screen.getByRole("textbox", { name }) as HTMLInputElement;

test("a permission card shows the tool, the full command and the reason, and takes the focus", () => {
  const { region, answers } = card("permission");
  expect(region.getAttribute("aria-label")).toBe("Permission request");
  expect(document.activeElement).toBe(region);
  expect(region.querySelector(".chat-card-tool")?.textContent).toBe("Bash");
  expect(region.querySelector("pre")?.textContent).toBe("rm -rf target");
  expect(screen.getByText("Bash commands need approval in the default mode")).toBeDefined();
  expect(region.querySelector("button kbd")).toBeNull();
  // Enter on the card allows; the answer goes once, then every button is off.
  fireEvent.keyDown(region, { key: "Enter" });
  fireEvent.keyDown(region, { key: "Enter" });
  fireEvent.click(button("Deny"));
  expect(answers()).toEqual([[4, "req_1", { kind: "allow" }]]);
  expect(button("Allow").disabled).toBe(true);
});

test("Esc or Deny denies a permission, with the message when one was written", () => {
  const { region, answers } = card("permission");
  fireEvent.keyDown(region, { key: "Escape" });
  expect(answers()).toEqual([[4, "req_1", { kind: "deny", message: null }]]);
  cleanup();

  const second = card("permission");
  fireEvent.change(text("Deny message"), { target: { value: " not now " } });
  fireEvent.click(button("Deny"));
  expect(second.answers()).toEqual([[4, "req_1", { kind: "deny", message: "not now" }]]);
});

test("Enter in the deny message denies with it; empty, it allows", () => {
  const { answers } = card("permission");
  const message = text("Deny message");
  // Shift+Enter, a button's own Enter and a composition do nothing.
  fireEvent.keyDown(message, { key: "Enter", shiftKey: true });
  fireEvent.keyDown(button("Allow"), { key: "Enter" });
  fireEvent.keyDown(message, { key: "Enter", isComposing: true });
  fireEvent.keyDown(message, { key: "a" });
  expect(answers()).toEqual([]);
  fireEvent.change(message, { target: { value: "use cargo clean" } });
  fireEvent.keyDown(message, { key: "Enter" });
  expect(answers()).toEqual([[4, "req_1", { kind: "deny", message: "use cargo clean" }]]);
  cleanup();

  const second = card("permission");
  fireEvent.keyDown(text("Deny message"), { key: "Enter" });
  fireEvent.click(button("Allow"));
  expect(second.answers()).toEqual([[4, "req_1", { kind: "allow" }]]);
});

test("a question card sends a label, the checked labels, or the free text, per question", () => {
  const { region, answers } = card("question");
  expect(region.getAttribute("aria-label")).toBe("Question");
  expect(screen.getByText("Which language should I greet you in?")).toBeDefined();
  expect(button("English").title).toBe("Greet in English");
  // Send waits for every question.
  expect(button("Send").disabled).toBe(true);
  expect(region.querySelector("button kbd")).toBeNull();
  fireEvent.keyDown(region, { key: "Enter" });
  fireEvent.click(button("English"));
  fireEvent.click(button("Portuguese"));
  expect(button("English").getAttribute("aria-pressed")).toBe("false");
  expect(button("Portuguese").getAttribute("aria-pressed")).toBe("true");
  const readme = screen.getByRole("checkbox", { name: "README.md" });
  const contributing = screen.getByRole("checkbox", { name: "CONTRIBUTING.md" });
  fireEvent.click(readme);
  fireEvent.click(contributing);
  fireEvent.click(readme);
  expect(answers()).toEqual([]);
  fireEvent.keyDown(contributing, { key: "Enter" });
  expect(answers()).toEqual([
    [4, "req_1", { kind: "answers", answers: [["Portuguese"], ["CONTRIBUTING.md"]] }],
  ]);
  cleanup();

  // "Other…" replaces the choice; Enter in it sends.
  const second = card("question");
  fireEvent.click(button("English"));
  fireEvent.change(text("Other answer to Language"), { target: { value: " Klingon " } });
  const files = text("Other answer to Files");
  fireEvent.change(files, { target: { value: "all" } });
  fireEvent.keyDown(files, { key: "Enter" });
  expect(second.answers()).toEqual([
    [4, "req_1", { kind: "answers", answers: [["Klingon"], ["all"]] }],
  ]);
});

test("Dismiss or Esc denies a question", () => {
  const { answers } = card("question");
  fireEvent.click(button("Dismiss"));
  expect(answers()).toEqual([[4, "req_1", { kind: "deny", message: null }]]);
  cleanup();
  const second = card("question");
  fireEvent.keyDown(text("Other answer to Files"), { key: "Escape" });
  expect(second.answers()).toEqual([[4, "req_1", { kind: "deny", message: null }]]);
});

test("a plan card approves, approves accepting edits, or keeps planning with feedback", () => {
  const { region, answers } = card("plan");
  expect(region.getAttribute("aria-label")).toBe("Plan approval");
  expect(region.querySelector("pre")?.textContent).toContain("## Add CONTRIBUTING.md");
  fireEvent.keyDown(region, { key: "Enter" });
  expect(answers()).toEqual([[4, "req_1", { kind: "approve_plan", accept_edits: false }]]);
  cleanup();

  const second = card("plan");
  fireEvent.click(button("Approve and accept edits"));
  expect(second.answers()).toEqual([[4, "req_1", { kind: "approve_plan", accept_edits: true }]]);
  cleanup();

  const third = card("plan");
  expect(button("Keep planning").disabled).toBe(true);
  const feedback = text("Feedback");
  fireEvent.change(feedback, { target: { value: "add a testing section" } });
  fireEvent.keyDown(feedback, { key: "Enter" });
  expect(third.answers()).toEqual([
    [4, "req_1", { kind: "keep_planning", feedback: "add a testing section" }],
  ]);
  cleanup();

  const fourth = card("plan");
  fireEvent.change(text("Feedback"), { target: { value: "shorter" } });
  fireEvent.click(button("Keep planning"));
  expect(fourth.answers()).toEqual([[4, "req_1", { kind: "keep_planning", feedback: "shorter" }]]);
});

test("the card leaves the focus in an input or an editable element", () => {
  for (const make of [
    () => document.createElement("input"),
    () => document.createElement("textarea"),
    () => {
      const div = document.createElement("div");
      div.contentEditable = "true";
      div.tabIndex = 0;
      return div;
    },
  ]) {
    const field = document.body.appendChild(make());
    field.focus();
    expect(document.activeElement).toBe(field);
    card("question");
    expect(document.activeElement).toBe(field);
    cleanup();
    field.remove();
  }
});
