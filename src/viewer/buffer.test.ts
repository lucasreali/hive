import { expect, test } from "bun:test";
import type { FileText } from "../store";
import {
  type EditBuffer,
  failed,
  fromDisk,
  isDirty,
  isFor,
  resolve,
  saved,
  startEdit,
  startSave,
  toText,
} from "./buffer";

const answer = (content: string | null, patch: Partial<FileText> = {}): FileText => ({
  worktree: "/w",
  path: "a.ts",
  content,
  base: null,
  version: content === null ? null : `v:${content}`,
  binary: false,
  too_large: false,
  error: null,
  ...patch,
});

const opened = (content = "one\n") => startEdit(answer(content)) as EditBuffer;
const typed = (b: EditBuffer, content: string): EditBuffer => ({ ...b, doc: toText(content) });
const text = (b: EditBuffer) => b.doc.toString();

test("a buffer starts from an answer with text, clean", () => {
  const b = opened();
  expect(b).toMatchObject({ worktree: "/w", path: "a.ts", version: "v:one\n", conflict: null });
  expect(text(b)).toBe("one\n");
  expect(isDirty(b)).toBe(false);
  expect(isDirty(typed(b, "two\n"))).toBe(true);
  for (const patch of [{ binary: true }, { too_large: true }]) {
    expect(startEdit(answer("x", patch))).toBeNull();
  }
  expect(startEdit(answer(null))).toBeNull();
  // "\r" is text, never a line break, so it saves unchanged.
  expect(toText("a\r\nb").toString()).toBe("a\r\nb");
  expect(toText("a\r\nb").lines).toBe(2);
  expect(isFor(b, { worktree: "/w", path: "a.ts" })).toBe(true);
  expect(isFor(b, { worktree: "/w", path: "b.ts" })).toBe(false);
  expect(isFor(b, { worktree: "/x", path: "a.ts" })).toBe(false);
});

test("a clean buffer reloads a new text on disk", () => {
  const b = fromDisk(opened(), answer("agent\n"));
  expect(text(b)).toBe("agent\n");
  expect(b.version).toBe("v:agent\n");
  expect(isDirty(b)).toBe(false);
  // Gone: the buffer empties, and any text in it is then unsaved.
  const gone = fromDisk(b, answer(null));
  expect(text(gone)).toBe("");
  expect([gone.version, gone.saved, isDirty(gone)]).toEqual([null, null, false]);
  expect(isDirty(typed(gone, "x"))).toBe(true);
  // Still gone: nothing changes.
  expect(fromDisk(gone, answer(null, { version: null }))).toEqual(gone);
});

test("a dirty buffer keeps its edits and shows the conflict", () => {
  const mine = typed(opened(), "mine\n");
  const b = fromDisk(mine, answer("agent\n"));
  expect(text(b)).toBe("mine\n");
  expect(b.conflict).toEqual({ content: "agent\n", version: "v:agent\n" });
  expect(b.version).toBe("v:one\n");
  // The disk back at the buffer's base (or equal to it under a new version) ends it.
  const back = fromDisk(b, answer("one\n", { version: "v2" }));
  expect([back.conflict, back.version, text(back)]).toEqual([null, "v2", "mine\n"]);
  // The disk holding the buffer's own text makes it clean at that version.
  const same = fromDisk(b, answer("mine\n"));
  expect([same.conflict, same.version, isDirty(same)]).toEqual([null, "v:mine\n", false]);
  // Deleted under edits: a conflict too.
  expect(fromDisk(mine, answer(null)).conflict).toEqual({ content: null, version: null });
});

test("binary or too large answers leave the buffer alone", () => {
  const b = typed(opened(), "mine\n");
  expect(fromDisk(b, answer(null, { binary: true, version: "x" }))).toBe(b);
  expect(fromDisk(b, answer(null, { too_large: true }))).toBe(b);
});

test("a save sends the text once and takes the new version", () => {
  const clean = opened();
  expect(startSave(clean)).toBeNull();
  const sending = startSave({ ...typed(clean, "two\n"), error: "old failure" }) as EditBuffer;
  expect(sending.saving?.toString()).toBe("two\n");
  expect(sending.error).toBeNull();
  expect(startSave(sending)).toBeNull(); // One save at a time.

  // Typing on while it is saved: the sent text is on disk, the rest still unsaved.
  const more = typed(sending, "two\nthree\n");
  const done = saved(more, "v:two\n");
  expect([done.saving, done.version, done.saved?.toString()]).toEqual([null, "v:two\n", "two\n"]);
  expect(isDirty(done)).toBe(true);
  expect(saved(done, "stray")).toBe(done);
});

test("a failed save says why; a conflict asks for the file again", () => {
  const sending = startSave(typed(opened(), "two\n")) as EditBuffer;
  const io = failed(sending, "io", "disk full");
  expect([io.saving, io.error, io.recheck]).toEqual([null, "disk full", 0]);
  const conflict = failed(sending, "conflict", "a.ts changed on disk");
  expect([conflict.error, conflict.recheck]).toEqual(["a.ts changed on disk", 1]);
});

test("the banner reloads the disk's text or keeps mine over it", () => {
  const b = fromDisk(typed(opened(), "mine\n"), answer("agent\n"));
  const reloaded = resolve({ ...b, error: "e" }, false);
  expect([text(reloaded), reloaded.version, reloaded.conflict, reloaded.error]).toEqual([
    "agent\n",
    "v:agent\n",
    null,
    null,
  ]);
  expect(isDirty(reloaded)).toBe(false);
  const kept = resolve(b, true);
  expect([text(kept), kept.version, kept.conflict]).toEqual(["mine\n", "v:agent\n", null]);
  expect(isDirty(kept)).toBe(true);
  // Deleted: reload empties it, keep mine creates the file on the next save.
  const deleted = fromDisk(typed(opened(), "mine\n"), answer(null));
  expect(text(resolve(deleted, false))).toBe("");
  const recreate = resolve(deleted, true);
  expect([recreate.version, isDirty(recreate)]).toEqual([null, true]);
  // Nothing to resolve.
  expect(resolve(kept, false)).toBe(kept);
});
