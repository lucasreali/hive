import { afterEach, beforeAll, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply, initialState, showTranscript, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus, mockTranscript } from "../transport/mock";
import { TerminalArea } from "./TerminalArea";

beforeAll(() => {
  // happy-dom has no layout: give the list its CSS size so the virtualizer shows entries.
  for (const [key, size] of [
    ["offsetHeight", 400],
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

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const at = { agent: "s", subagent: "a1" };

function shown() {
  const watch = spyOn(transport, "watchTranscript");
  const unwatch = spyOn(transport, "unwatchTranscript");
  const view = render(<TerminalArea />);
  act(() => {
    useHive.setState({ tabs: [{ id: 1, cwd: "/w" }], activeTab: 1 });
    apply({
      type: "agent_detected",
      channel: 1,
      id: "s",
      project: null,
      worktree: "/w",
      cwd: "/w",
    });
    apply({
      type: "agent_state",
      id: "s",
      ...agentStatus("with_subagents"),
      subagents: [
        {
          id: "a1",
          agent_type: "Explore",
          state: "working",
          worktree: null,
          activity: null,
          since_ms: 0,
        },
      ],
    });
    showTranscript("s", "a1");
  });
  return { watch, unwatch, view };
}

const texts = () =>
  [...document.querySelectorAll(".transcript-entry")].map((e) => [
    e.getAttribute("data-role"),
    e.querySelector(".transcript-role")?.textContent,
    e.querySelector(".transcript-text")?.textContent,
  ]);

test("a subagent's conversation shows read-only in place of the terminal while followed", () => {
  const { watch, unwatch, view } = shown();
  expect(watch.mock.calls).toEqual([["s", "a1"]]);
  const section = screen.getByRole("region", { name: "Subagent conversation" });
  expect(section.querySelector(".path")?.textContent).toBe("subagent: Exploreworking");
  expect(section.querySelector(".state-icon")?.getAttribute("data-state")).toBe("working");
  expect(screen.getByText("Loading the conversation…")).toBeDefined();
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(true);
  // No terminal tab is the active one meanwhile.
  expect(screen.getByRole("tab").getAttribute("aria-selected")).toBe("false");

  act(() => apply({ type: "transcript", ...at, entries: [], truncated: false }));
  expect(screen.getByText("Nothing written yet.")).toBeDefined();
  act(() => apply({ type: "transcript", ...at, entries: mockTranscript("a1"), truncated: true }));
  expect(screen.getByText("Earlier messages are left out.")).toBeDefined();
  expect(texts()).toEqual([
    ["user", "Prompt", "Find where the login form is handled (a1)."],
    ["assistant", "Subagent", "I'll search the code for the login handler."],
    ["tool", "Grep", '{"pattern":"login","path":"src"}'],
    ["assistant", "Subagent", "The login form posts to /api/session in src/auth.ts."],
  ]);
  const more = { role: "assistant" as const, text: "Done.", tool: null };
  act(() => apply({ type: "transcript_appended", ...at, entries: [more] }));
  expect(texts().at(-1)).toEqual(["assistant", "Subagent", "Done."]);
  // Another subagent's conversation is not shown here.
  act(() =>
    apply({ type: "transcript", agent: "s", subagent: "b", entries: [], truncated: false }),
  );
  expect(screen.getByText("Loading the conversation…")).toBeDefined();

  // A subagent that left its agent's list has ended.
  act(() => apply({ type: "agent_state", id: "s", ...agentStatus("idle"), subagents: [] }));
  expect(section.querySelector(".path")?.textContent).toBe("subagent: unknownended");

  fireEvent.click(screen.getByRole("button", { name: "Back to terminal" }));
  expect(screen.queryByRole("region", { name: "Subagent conversation" })).toBeNull();
  expect(unwatch.mock.calls).toEqual([["s", "a1"]]);
  expect((document.querySelector(".terminal-host") as HTMLElement).hidden).toBe(false);
  expect(screen.getByRole("tab").getAttribute("aria-selected")).toBe("true");
  view.unmount();
});
