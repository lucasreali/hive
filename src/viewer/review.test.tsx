import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply, type HiveState, initialState, useHive } from "../store";
import { closeTerminal, mountTerminals, openTerminal, terminal } from "../terminals";
import { transport } from "../transport";
import {
  CommentButton,
  CommentInput,
  deleteComment,
  endComment,
  ReviewList,
  reviewTarget,
  reviewText,
  sendReview,
  startComment,
} from "./review";

beforeAll(async () => {
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

const wt = "/home/u/shop";
const a = { path: "src/a.ts", from: 3, to: 5, text: "why?" };
const b = { path: "docs/my notes.md", from: 7, to: 7, text: "typo" };

test("a comment opens on the shown file's selected lines and is kept per worktree", () => {
  startComment();
  expect(useHive.getState().commenting).toBeNull();
  useHive.setState({ openFile: { worktree: wt, path: "src/a.ts" }, fileShown: true });
  startComment();
  expect(useHive.getState().commenting).toBeNull();
  useHive.setState({ selectedLines: { from: 3, to: 5 }, fileShown: false });
  startComment();
  expect(useHive.getState().commenting).toBeNull();
  useHive.setState({ fileShown: true });
  startComment();
  expect(useHive.getState().commenting).toEqual({ worktree: wt, path: "src/a.ts", from: 3, to: 5 });

  // An empty comment is dropped; a comment is trimmed and appended to its worktree's.
  endComment("  ");
  expect(useHive.getState()).toMatchObject({ commenting: null, comments: {} });
  endComment("ignored: no input open");
  expect(useHive.getState().comments).toEqual({});
  startComment();
  endComment(" why? ");
  startComment();
  endComment("again");
  expect(useHive.getState().comments).toEqual({ [wt]: [a, { ...a, text: "again" }] });

  deleteComment(wt, 0);
  expect(useHive.getState().comments).toEqual({ [wt]: [{ ...a, text: "again" }] });
  deleteComment("/elsewhere", 0);
  expect(useHive.getState().comments["/elsewhere"]).toEqual([]);
});

test("the review is one line per comment: its reference, then the comment", () => {
  expect(reviewText([a, b])).toBe(
    '@src/a.ts (lines 3–5) — why?\n@"docs/my notes.md" (line 7) — typo',
  );
});

const ready: Partial<HiveState> = {
  openFile: { worktree: wt, path: "src/a.ts" },
  comments: { [wt]: [a, b] },
  tabs: [
    { id: 1, cwd: "/home/u/other" },
    { id: 2, cwd: wt },
  ],
  activeTab: 2,
};
const target = (patch: Partial<HiveState>) => reviewTarget({ ...initialState, ...ready, ...patch });

test("the review goes where a reference would, or it says why not", () => {
  expect(target({})).toEqual({ terminal: 2, worktree: wt, text: reviewText([a, b]) });
  expect(target({ openFile: null })).toEqual({ why: "No comments to send" });
  expect(target({ comments: {} })).toEqual({ why: "No comments to send" });
  expect(target({ comments: { [wt]: [] } })).toEqual({ why: "No comments to send" });
  expect(target({ activeTab: 1 })).toEqual({ why: "The active terminal is in another worktree" });
  expect(target({ activeTab: null })).toEqual({ why: "No terminal open" });
});

test("sending pastes the review without Enter, shows the terminal and drops the comments", async () => {
  const write = spyOn(transport, "writeTerminal");
  // Nothing to send; then a target whose terminal has no xterm: nothing happens.
  sendReview();
  useHive.setState({ ...ready, fileShown: true });
  sendReview();
  expect(write).not.toHaveBeenCalled();
  expect(useHive.getState().comments[wt]).toHaveLength(2);

  useHive.setState({ tabs: [], activeTab: null });
  const host = document.body.appendChild(document.createElement("div"));
  const unmount = mountTerminals(host);
  const id = await act(() => openTerminal(wt));
  await act(() => new Promise((resolve) => setTimeout(resolve, 10)));
  const other = { path: "b.ts", from: 1, to: 1, text: "kept" };
  useHive.setState({ comments: { [wt]: [a, b], "/other": [other] }, fileShown: true });
  sendReview();
  // Not in bracketed paste mode, xterm turns newlines into carriage returns, as for Ctrl+Shift+V.
  expect(write).toHaveBeenCalledWith(id, reviewText([a, b]).replace("\n", "\r"));
  expect(write).not.toHaveBeenCalledWith(id, expect.stringContaining("typo\r"));
  expect(useHive.getState()).toMatchObject({
    fileShown: false,
    comments: { [wt]: [], "/other": [other] },
  });
  expect(document.activeElement).toBe(terminal(id)?.textarea as Element);
  write.mockRestore();
  unmount();
  host.remove();
});

test("the input saves on Enter and cancels on Escape, only for its own file", () => {
  useHive.setState({ commenting: { worktree: wt, path: "src/a.ts", from: 3, to: 5 } });
  const { rerender } = render(<CommentInput worktree={wt} path="src/b.ts" />);
  expect(screen.queryByRole("textbox")).toBeNull();
  rerender(<CommentInput worktree="/other" path="src/a.ts" />);
  expect(screen.queryByRole("textbox")).toBeNull();
  rerender(<CommentInput worktree={wt} path="src/a.ts" />);
  const input = screen.getByLabelText("Lines 3–5") as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: "why?" } });
  fireEvent.keyDown(input, { key: "a" });
  expect(useHive.getState().comments).toEqual({});
  fireEvent.keyDown(input, { key: "Enter" });
  expect(useHive.getState()).toMatchObject({ commenting: null, comments: { [wt]: [a] } });

  act(() => useHive.setState({ commenting: { worktree: wt, path: "src/a.ts", from: 9, to: 9 } }));
  const one = screen.getByLabelText("Line 9") as HTMLInputElement;
  fireEvent.change(one, { target: { value: "dropped" } });
  fireEvent.keyDown(one, { key: "Escape" });
  expect(useHive.getState()).toMatchObject({ commenting: null, comments: { [wt]: [a] } });
  expect(screen.queryByRole("textbox")).toBeNull();
});

test("the list shows the comments, deletes them, and sends them when it can", () => {
  const { rerender } = render(<ReviewList worktree={wt} />);
  expect(screen.queryByRole("region", { name: "Review comments" })).toBeNull();
  act(() => useHive.setState({ comments: { [wt]: [a, b] } }));
  const list = screen.getByRole("region", { name: "Review comments" });
  expect([...list.querySelectorAll("li")].map((li) => li.textContent)).toEqual([
    "src/a.ts:3–5why?",
    "docs/my notes.md:7typo",
  ]);
  const send = screen.getByRole("button", { name: "Send review (2)" }) as HTMLButtonElement;
  expect(send.disabled).toBe(true);
  expect(send.title).toBe("No comments to send");
  act(() => useHive.setState(ready));
  expect(send.disabled).toBe(false);
  expect(send.title).toBe("Paste the comments into the terminal, without Enter");

  fireEvent.click(screen.getAllByRole("button", { name: "Delete comment" })[0] as HTMLElement);
  expect(useHive.getState().comments[wt]).toEqual([b]);
  screen.getByRole("button", { name: "Send review (1)" });
  act(() => useHive.setState({ comments: { [wt]: [] } }));
  rerender(<ReviewList worktree={wt} />);
  expect(screen.queryByRole("region", { name: "Review comments" })).toBeNull();
});

test("the Comment button needs selected lines and opens the input", () => {
  render(<CommentButton />);
  const button = screen.getByRole("button", { name: "Comment" }) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  expect(button.title).toBe("Select lines to comment on them");
  act(() =>
    useHive.setState({
      openFile: { worktree: wt, path: "src/a.ts" },
      fileShown: true,
      selectedLines: { from: 4, to: 4 },
    }),
  );
  expect(button.disabled).toBe(false);
  expect(button.title).toBe("Comment on the selected lines (Ctrl+Shift+M)");
  fireEvent.click(button);
  expect(useHive.getState().commenting).toEqual({ worktree: wt, path: "src/a.ts", from: 4, to: 4 });
});
