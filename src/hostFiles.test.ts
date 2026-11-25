import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { onMessage } from "./connect";
import type { FileText, HostMessage } from "./protocol";
import { initialState, setEdit, setOpenFile, showHost, useHive } from "./store";
import { barItems } from "./tabs";
import { transport } from "./transport";
import { startEdit, toText } from "./viewer/buffer";
import { saveOpenFile } from "./viewer/save";

// 26.18 review: open files, edit buffers and pending saves are their host's. A host's messages
// never reach another's tabs, and a save never goes to another host.

const HOST = "gpu-box";
const at = { worktree: "/r", path: "a.ts" };
const text: FileText = {
  ...at,
  content: "local\n",
  base: null,
  version: "v1",
  binary: false,
  too_large: false,
  error: null,
  media: null,
};
const remote = (m: object) => onMessage({ ...m, host: HOST } as HostMessage);

beforeEach(() => {
  useHive.setState(initialState, true);
  // This Mac's file /r/a.ts open, edited and not saved; the host linked.
  onMessage({ type: "host_link", host: HOST, state: "connected" });
  useHive.setState({ selection: "/r" });
  setOpenFile(at, true);
  onMessage({ type: "file", ...text });
  const edit = startEdit(text);
  if (edit) setEdit({ ...edit, doc: toText("edited\n") });
});
afterEach(() => useHive.setState(initialState, true));

/** This Mac's open file as the store keeps it, shown or not. */
const local = () => {
  const s = useHive.getState();
  return s.host === null ? s : { ...s, ...s.stash[""] };
};

test("a host shown with no projects shows none of this Mac's file tabs", () => {
  showHost(HOST);
  const s = useHive.getState();
  expect([s.openFile, s.edit, s.openFiles]).toEqual([null, null, []]);
  expect(barItems({ ...s, selection: null })).toEqual([]);
  showHost(null);
  expect(useHive.getState().openFile).toEqual(at);
  expect(useHive.getState().edit?.doc.toString()).toBe("edited\n");
});

for (const shown of [null, HOST]) {
  test(`a remote's file, save, rename or deletion naming this Mac's open file changes nothing (${shown ?? "this Mac"} shown)`, () => {
    showHost(shown);
    const before = local();
    const kept = { openFiles: before.openFiles, edit: before.edit, file: before.file };
    remote({ type: "file", ...text, content: "remote\n", version: "v9" });
    remote({ type: "file_saved", ...at, version: "v9" });
    remote({ type: "save_failed", ...at, error: "conflict", message: "x" });
    remote({ type: "file_renamed", worktree: "/r", path: "a.ts", to: "b.ts" });
    remote({ type: "file_deleted", ...at });
    remote({ type: "project_removed", id: "/r" });
    const after = local();
    expect({ openFiles: after.openFiles, edit: after.edit, file: after.file }).toEqual(kept);
    expect(after.edit?.doc.toString()).toBe("edited\n");
  });
}

test("Ctrl+S on this Mac's file never goes to a remote shown: it saves only on its own host", () => {
  const save = spyOn(transport, "saveFile").mockResolvedValue();
  showHost(HOST);
  saveOpenFile();
  expect(save).not.toHaveBeenCalled();
  showHost(null);
  saveOpenFile();
  expect(save.mock.calls).toEqual([["/r", "a.ts", "edited\n", "v1"]]);
  save.mockRestore();
});

test("a remote's save failing after the host was removed brings none of its data back", async () => {
  const save = spyOn(transport, "saveFile").mockRejectedValue(new Error("not connected"));
  showHost(HOST);
  setOpenFile(at, true);
  remote({ type: "file", ...text, content: "remote\n" });
  const theirs = startEdit({ ...text, content: "remote\n" });
  if (theirs) setEdit({ ...theirs, doc: toText("theirs\n") });
  saveOpenFile();
  expect(save).toHaveBeenCalledTimes(1);
  onMessage({ type: "host_link", host: HOST, state: "removed" });
  await Bun.sleep(0);
  const s = useHive.getState();
  expect([s.host, HOST in s.stash, HOST in s.links]).toEqual([null, false, false]);
  expect(s.edit?.doc.toString()).toBe("edited\n");
  save.mockRestore();
});

test("a save that cannot be sent marks its own host's buffer, even once another is shown", async () => {
  const save = spyOn(transport, "saveFile").mockRejectedValue(new Error("not connected"));
  saveOpenFile();
  // The remote, the same path open there, is shown before the failure comes back.
  showHost(HOST);
  setOpenFile(at, true);
  remote({ type: "file", ...text, content: "remote\n" });
  const theirs = startEdit({ ...text, content: "remote\n" });
  if (theirs) setEdit(theirs);
  await Bun.sleep(0);
  expect(useHive.getState().edit?.error).toBeNull();
  expect(local().edit).toMatchObject({ saving: null, error: "Error: not connected" });
  save.mockRestore();
});
