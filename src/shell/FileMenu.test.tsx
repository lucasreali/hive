import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { apply, initialState, openFileMenu, useHive } from "../store";
import { transport } from "../transport";
import { ConfirmDialog } from "./ConfirmDialog";
import { askDelete, FileMenu, FileNameDialog } from "./FileMenu";

afterEach(() => {
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

function Shown() {
  const modal = useHive((s) => s.modal);
  return (
    <>
      <FileMenu />
      {modal === "file-name" && <FileNameDialog />}
      {modal === "confirm" && <ConfirmDialog />}
    </>
  );
}

const items = () => screen.queryAllByRole("menuitem").map((i) => i.textContent);
const field = () => screen.getByLabelText("Name") as HTMLInputElement;
const submit = () => screen.getByRole("button", { name: /^(Create|Rename)$/ }) as HTMLButtonElement;
const type = (name: string) => fireEvent.change(field(), { target: { value: name } });

test("a file's menu renames it in its folder; the service's refusal shows until the next edit", () => {
  const rename = spyOn(transport, "renameFile").mockResolvedValue();
  render(<Shown />);
  expect(items()).toEqual([]);
  act(() => openFileMenu({ worktree: "/w", folder: "src", path: "src/a.ts", x: 1, y: 2 }));
  expect(items()).toEqual(["New File…", "New Folder…", "Rename…", "Delete…"]);
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename…" }));
  expect(useHive.getState().fileMenu).toBeNull();
  expect(screen.getByRole("heading").textContent).toBe("Rename file");
  expect(field().value).toBe("a.ts");
  expect(submit().disabled).toBe(true);
  expect(document.querySelector("dialog button kbd")).toBeNull();
  fireEvent.submit(field());
  expect(rename).not.toHaveBeenCalled();
  type("b.ts");
  fireEvent.submit(field());
  expect(rename).toHaveBeenCalledWith("/w", "src/a.ts", "b.ts");

  // Another worktree's answer is not this dialog's.
  act(() => apply({ type: "file_op_failed", worktree: "/x", message: "no" }));
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => apply({ type: "file_op_failed", worktree: "/w", message: "b.ts already exists" }));
  expect(screen.getByRole("alert").textContent).toBe("b.ts already exists");
  expect(field().getAttribute("aria-invalid")).toBe("true");
  type("c.ts");
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => apply({ type: "file_renamed", worktree: "/w", path: "src/a.ts", to: "src/c.ts" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useHive.getState().fileDialog).toBeNull();
});

test("a folder's menu renames it where it is", () => {
  const rename = spyOn(transport, "renameFile").mockResolvedValue();
  render(<Shown />);
  act(() => openFileMenu({ worktree: "/w", folder: "src/lib", path: "src/lib", x: 1, y: 2 }));
  expect(items()).toEqual(["New File…", "New Folder…", "Rename…", "Delete…"]);
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename…" }));
  expect(screen.getByRole("heading").textContent).toBe("Rename folder");
  expect(field().value).toBe("lib");
  // Where the folder is, not the folder itself.
  expect(screen.getByText("src")).toBeTruthy();
  type("core");
  fireEvent.submit(field());
  expect(rename).toHaveBeenCalledWith("/w", "src/lib", "core");
  // A top folder is in the root.
  act(() => useHive.setState({ modal: null, fileDialog: null }));
  act(() => openFileMenu({ worktree: "/w", folder: "src", path: "src", x: 1, y: 2 }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename…" }));
  expect(screen.getByText("(worktree root)")).toBeTruthy();
});

test("a folder's menu creates in that folder; the root is named as such and has no rename", () => {
  const create = spyOn(transport, "createFile").mockResolvedValue();
  render(<Shown />);
  act(() => openFileMenu({ worktree: "/w", folder: "src", path: "src", x: 1, y: 2 }));
  fireEvent.click(screen.getByRole("menuitem", { name: "New File…" }));
  expect(screen.getByRole("heading").textContent).toBe("New file");
  expect(screen.getByText("src")).toBeTruthy();
  expect([field().value, submit().disabled]).toEqual(["", true]);
  type("x.ts");
  fireEvent.click(submit());
  expect(create).toHaveBeenCalledWith("/w", "src", "x.ts");
  fireEvent.click(screen.getByRole("button", { name: /^Cancel/ }));
  expect(useHive.getState()).toMatchObject({ modal: null, fileDialog: null });

  act(() => openFileMenu({ worktree: "/w", folder: "", path: null, x: 1, y: 2 }));
  expect(items()).toEqual(["New File…", "New Folder…"]);
  fireEvent.click(screen.getByRole("menuitem", { name: "New File…" }));
  expect(screen.getByText("(worktree root)")).toBeTruthy();
  fireEvent.click(screen.getByTitle("Close (Esc)"));
  expect(useHive.getState().modal).toBeNull();
});

test("Delete… asks first, naming the entry and a folder's files; Cancel deletes nothing", () => {
  const remove = spyOn(transport, "deleteFile").mockResolvedValue();
  const files = ["src/a.ts", "src/lib/b.ts", "srcx/c.ts", "top.ts"];
  apply({ type: "files", path: "/w", files, truncated: false });
  render(<Shown />);
  act(() => openFileMenu({ worktree: "/w", folder: "src", path: "src", x: 1, y: 2 }));
  expect(items()).toEqual(["New File…", "New Folder…", "Rename…", "Delete…"]);
  fireEvent.click(screen.getByRole("menuitem", { name: "Delete…" }));
  expect(useHive.getState().fileMenu).toBeNull();
  expect(screen.getByRole("heading").textContent).toBe("Delete folder?");
  const lost =
    "Nothing goes to a trash: git can bring back tracked files, but untracked and ignored ones are lost for good.";
  expect(screen.getByText(/^Delete the folder/).textContent).toBe(
    `Delete the folder src and the 2 files in it? ${lost}`,
  );
  // Cancel has the focus: Enter or a click on it deletes nothing.
  expect(document.activeElement?.textContent).toBe("Cancel");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  expect(remove).not.toHaveBeenCalled();

  act(() => openFileMenu({ worktree: "/w", folder: "", path: "top.ts", x: 1, y: 2 }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Delete…" }));
  expect(screen.getByRole("heading").textContent).toBe("Delete file?");
  expect(screen.getByText(/^Delete top/).textContent).toBe(`Delete top.ts? ${lost}`);
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(remove).toHaveBeenCalledWith("/w", "top.ts");

  // One file; another worktree's folder is not counted from this listing.
  act(() => askDelete({ worktree: "/w", folder: "srcx", path: "srcx" }));
  expect(useHive.getState().question?.text).toStartWith(
    "Delete the folder srcx and the 1 file in it?",
  );
  act(() => askDelete({ worktree: "/x", folder: "d", path: "d" }));
  expect(useHive.getState().question?.text).toStartWith(
    "Delete the folder d and the 0 files in it?",
  );
  // Nothing to delete without a path (the root, a deleted file).
  act(() => useHive.setState({ modal: null, question: null }));
  askDelete({ worktree: "/w", folder: "", path: null });
  expect(useHive.getState().question).toBeNull();
});

test("New Folder… asks for a name and creates it in the menu's folder", () => {
  const create = spyOn(transport, "createFolder").mockResolvedValue();
  render(<Shown />);
  act(() => openFileMenu({ worktree: "/w", folder: "src", path: "src/a.ts", x: 1, y: 2 }));
  fireEvent.click(screen.getByRole("menuitem", { name: "New Folder…" }));
  expect(screen.getByRole("heading").textContent).toBe("New folder");
  expect([field().value, submit().disabled]).toEqual(["", true]);
  type("lib");
  fireEvent.submit(field());
  expect(create).toHaveBeenCalledWith("/w", "src", "lib");
  act(() => apply({ type: "file_op_failed", worktree: "/w", message: "lib already exists" }));
  expect(screen.getByRole("alert").textContent).toBe("lib already exists");
  act(() => apply({ type: "folder_created", worktree: "/w", path: "src/lib" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
