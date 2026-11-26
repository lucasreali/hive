import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { detachAfterAll } from "../../test/transport";
import type { Project, ReviewComment } from "../protocol";
import { apply } from "../reduce";
import { type HiveState, initialState, useHive } from "../store";
import { closeTerminal, mountTerminals, openTerminal, terminal } from "../terminals";
import { transport } from "../transport";
import {
  CommentButton,
  CommentInput,
  CommentsView,
  commentsTarget,
  endComment,
  openComments,
  reviewText,
  sendable,
  sendComments,
  sendOpenComments,
  startComment,
} from "./review";

detachAfterAll();

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
const comment = (patch: Partial<ReviewComment>): ReviewComment => ({
  id: 1,
  file: "src/a.ts",
  from: 3,
  to: 5,
  body: "why?",
  code: ["let a = 1;"],
  before: [],
  after: [],
  author: "ana",
  ms: Date.now() - 5 * 60_000,
  resolved: false,
  outdated: false,
  ...patch,
});
const a = comment({});
const b = comment({ id: 2, file: "docs/my notes.md", from: 7, to: 7, body: "typo", code: [] });
const resolved = comment({ id: 3, body: "done", resolved: true });
const outdated = comment({ id: 4, body: "gone", outdated: true });

test("a comment opens on the shown file's selected lines and goes to the service", () => {
  const add = spyOn(transport, "addComment").mockImplementation(async () => {});
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

  // An empty comment is dropped; a comment is trimmed and sent.
  endComment("  ");
  expect(useHive.getState().commenting).toBeNull();
  endComment("ignored: no input open");
  expect(add).not.toHaveBeenCalled();
  startComment();
  endComment(" why? ");
  expect(add).toHaveBeenCalledWith(wt, "src/a.ts", 3, 5, "why?");
  expect(useHive.getState().commenting).toBeNull();
  add.mockRestore();
});

test("the review is one line per comment: its reference, then the comment", () => {
  expect(reviewText([a, b])).toBe(
    '@src/a.ts (lines 3–5) — why?\n@"docs/my notes.md" (line 7) — typo',
  );
  expect([a, b, resolved, outdated].map(sendable)).toEqual([true, true, false, false]);
});

const ready: Partial<HiveState> = {
  comments: { [wt]: [a, b, resolved, outdated] },
  tabs: [
    { id: 1, cwd: "/home/u/other" },
    { id: 2, cwd: wt },
  ],
  activeTab: 2,
};
const target = (patch: Partial<HiveState>, comments = [a, b]) =>
  commentsTarget({ ...initialState, ...ready, ...patch }, wt, comments);

test("comments go where a reference would, or it says why not", () => {
  expect(target({})).toEqual({ terminal: 2, text: reviewText([a, b]) });
  expect(target({}, [])).toEqual({ why: "No open comments to send" });
  expect(target({ activeTab: 1 })).toEqual({ why: "The active terminal is in another worktree" });
  expect(target({ activeTab: null })).toEqual({ why: "No terminal open" });
});

test("the open comments are the shown worktree's, open and still there", () => {
  expect(openComments(initialState)).toEqual({ worktree: "", comments: [] });
  const project = {
    id: wt,
    name: "shop",
    group: false,
    folder: false,
    worktrees: [{ id: wt, path: wt, name: "shop" }],
  } as unknown as Project;
  const s = { ...initialState, ...ready, projects: { [wt]: project }, selection: wt };
  expect(openComments(s)).toEqual({ worktree: wt, comments: [a, b] });
});

test("sending pastes the comments without Enter, shows the terminal and keeps them", async () => {
  const write = spyOn(transport, "writeTerminal");
  // Nothing to send; then a target whose terminal has no xterm: nothing happens.
  sendOpenComments();
  useHive.setState({ ...ready, fileShown: true });
  sendComments(wt, [a, b]);
  expect(write).not.toHaveBeenCalled();

  useHive.setState({ tabs: [], activeTab: null });
  const host = document.body.appendChild(document.createElement("div"));
  const unmount = mountTerminals(host);
  const id = await act(() => openTerminal(wt));
  await act(() => new Promise((resolve) => setTimeout(resolve, 10)));
  useHive.setState({ fileShown: true });
  sendComments(wt, [a, b]);
  // Not in bracketed paste mode, xterm turns newlines into carriage returns, as for Ctrl+Shift+V.
  expect(write).toHaveBeenCalledWith(id, reviewText([a, b]).replace("\n", "\r"));
  expect(write).not.toHaveBeenCalledWith(id, expect.stringContaining("typo\r"));
  expect(useHive.getState()).toMatchObject({
    fileShown: false,
    comments: { [wt]: [a, b, resolved, outdated] },
  });
  expect(document.activeElement).toBe(terminal(id)?.textarea as Element);
  write.mockRestore();
  unmount();
  host.remove();
});

test("the input saves on Enter and cancels on Escape, only for its own file", () => {
  const add = spyOn(transport, "addComment").mockImplementation(async () => {});
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
  // Enter and Esc ending a composition (a dead key, an IME; 17.14) are the composition's.
  fireEvent.keyDown(input, { key: "Enter", isComposing: true });
  fireEvent.keyDown(input, { key: "Escape", isComposing: true });
  fireEvent.keyDown(input, { key: "Enter", keyCode: 229 });
  expect(add).not.toHaveBeenCalled();
  expect(useHive.getState().commenting).not.toBeNull();
  fireEvent.keyDown(input, { key: "Enter" });
  expect(add).toHaveBeenCalledWith(wt, "src/a.ts", 3, 5, "why?");
  expect(useHive.getState().commenting).toBeNull();

  act(() => useHive.setState({ commenting: { worktree: wt, path: "src/a.ts", from: 9, to: 9 } }));
  const one = screen.getByLabelText("Line 9") as HTMLInputElement;
  fireEvent.change(one, { target: { value: "dropped" } });
  fireEvent.keyDown(one, { key: "Escape" });
  expect(add).toHaveBeenCalledTimes(1);
  expect(useHive.getState().commenting).toBeNull();
  expect(screen.queryByRole("textbox")).toBeNull();
  add.mockRestore();
});

test("the list groups the comments and sends, resolves, reopens and deletes them", () => {
  const resolve = spyOn(transport, "resolveComment").mockImplementation(async () => {});
  const remove = spyOn(transport, "deleteComment").mockImplementation(async () => {});
  render(<CommentsView worktree={wt} />);
  expect(
    screen.getByText("No comments. Select lines of a file and press Comment (Ctrl+Shift+M)."),
  ).toBeTruthy();
  act(() => useHive.setState({ comments: { [wt]: [a, b, resolved, outdated] } }));
  const group = (name: string) => screen.getByRole("region", { name });
  const items = (name: string) =>
    [...group(name).querySelectorAll("li")].map(
      (li) => li.querySelector(".comment-body")?.textContent,
    );
  expect(screen.getByText("Open (2)")).toBeTruthy();
  expect(items("Open")).toEqual(["why?", "typo"]);
  expect(items("Outdated")).toEqual(["gone"]);
  expect(items("Resolved")).toEqual(["done"]);
  const first = within(group("Open")).getAllByRole("listitem")[0] as HTMLElement;
  expect(within(first).getByText("src/a.ts:3–5")).toBeTruthy();
  expect(within(first).getByText("ana · 5m ago")).toBeTruthy();
  expect(first.querySelector(".comment-code")?.textContent).toBe("let a = 1;");
  // A comment with no code shows none; an outdated one and a resolved one have no Send.
  expect(
    within(group("Open")).getAllByRole("listitem")[1]?.querySelector(".comment-code"),
  ).toBeNull();
  expect(within(group("Outdated")).queryByRole("button", { name: "Send" })).toBeNull();
  expect(within(group("Resolved")).queryByRole("button", { name: "Send" })).toBeNull();

  // No terminal of the worktree: Send says why.
  const all = screen.getByRole("button", { name: "Send all open (2)" }) as HTMLButtonElement;
  const one = within(first).getByRole("button", { name: "Send" }) as HTMLButtonElement;
  expect([all.disabled, all.title]).toEqual([true, "No terminal open"]);
  expect([one.disabled, one.title]).toEqual([true, "No terminal open"]);
  act(() => useHive.setState({ tabs: ready.tabs, activeTab: 2 }));
  expect([all.disabled, all.title]).toEqual([
    false,
    "Paste the open comments into the terminal, without Enter",
  ]);
  expect(one.title).toBe("Paste the comment into the terminal, without Enter");
  // No xterm in this test: nothing is pasted, nothing breaks.
  fireEvent.click(all);
  fireEvent.click(one);

  fireEvent.click(within(first).getByRole("button", { name: "Resolve" }));
  expect(resolve).toHaveBeenCalledWith(wt, 1, true);
  fireEvent.click(within(group("Resolved")).getByRole("button", { name: "Reopen" }));
  expect(resolve).toHaveBeenCalledWith(wt, 3, false);
  fireEvent.click(within(group("Outdated")).getByRole("button", { name: "Delete comment" }));
  expect(remove).toHaveBeenCalledWith(wt, 4);

  // Its place opens the file there.
  fireEvent.click(within(first).getByText("src/a.ts:3–5"));
  expect(useHive.getState().openFile).toEqual({ worktree: wt, path: "src/a.ts" });
  expect(within(group("Outdated")).getByText("src/a.ts:3–5").title).toBe(
    "Its code is gone: where it was",
  );
  resolve.mockRestore();
  remove.mockRestore();
});

test("comments added through the service are listed, moved and resolved", async () => {
  const shop = Object.values(useHive.getState().projects ?? {});
  expect(shop).toEqual([]);
  // The mock's own worktree and file.
  await transport.listProjects();
  await new Promise((resolve) => setTimeout(resolve, 0));
  const project = Object.values(useHive.getState().projects ?? {})[0] as Project;
  const path = project.worktrees[0]?.path as string;
  await transport.addComment(path, "README.md", 1, 1, "intro?");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const [added] = useHive.getState().comments[path] ?? [];
  expect(added).toMatchObject({ id: 1, file: "README.md", from: 1, to: 1, body: "intro?" });
  await transport.resolveComment(path, 1, true);
  await transport.deleteComment(path, 1);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(useHive.getState().comments[path]).toEqual([]);
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
