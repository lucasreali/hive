import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_CLAUDE_DIR, mockDirs } from "../transport/mock";
import { FolderField, LIST_DELAY_MS, useFolderListing } from "./FolderField";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

/** A field on its own, its value shown beside it, the service's answers given by `answer`. */
function Field(props: { hidden?: boolean; home?: boolean; onBlur?: () => void }) {
  const [value, setValue] = useState("");
  const listing = useFolderListing(value);
  return (
    <>
      <FolderField
        id="f"
        aria-label="Folder"
        value={value}
        onChange={setValue}
        placeholder="p"
        {...props}
      />
      <output>{listing?.path === value && listing.claude ? "claude" : ""}</output>
    </>
  );
}

const field = () => screen.getByLabelText("Folder") as HTMLInputElement;
const folders = () =>
  [...screen.getByRole("list", { name: "Folders" }).querySelectorAll("li")].map(
    (li) => li.textContent,
  );
const type = (value: string) => fireEvent.change(field(), { target: { value } });
/** The service's answer to the last listing asked. */
const answer = (asked: ReturnType<typeof spyOn<typeof transport, "listDirs">>) => {
  const [path, windows, hidden] = asked.mock.calls.at(-1) as [string, boolean, boolean];
  act(() => apply({ type: "dirs", ...mockDirs(path, windows, false, hidden) }));
};

test("lists once typing pauses, picks by click, goes up, and keeps a typed path", async () => {
  const asked = spyOn(transport, "listDirs").mockResolvedValue();
  try {
    render(<Field />);
    // Empty: the home folder is listed, the field stays empty.
    await waitFor(() => expect(asked).toHaveBeenCalledWith("", false, false));
    answer(asked);
    expect(field().value).toBe("");
    expect(folders()).toEqual(["/home/", "dotfiles", "Downloads", "projects", "work"]);
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    expect(field().value).toBe("/home/user/projects/");
    expect(document.activeElement).toBe(field());
    await waitFor(() =>
      expect(asked).toHaveBeenLastCalledWith("/home/user/projects/", false, false),
    );
    answer(asked);
    expect(folders()).toEqual(["/home/user/", "api", "notes", "shop"]);
    fireEvent.click(screen.getByTitle("Up a level"));
    expect(field().value).toBe("/home/user/");
    // A typed path filters and stays as typed.
    type("/home/user/projects/no");
    await waitFor(() =>
      expect(asked).toHaveBeenLastCalledWith("/home/user/projects/no", false, false),
    );
    answer(asked);
    expect(folders()).toEqual(["/home/user/", "notes"]);
    expect(field().value).toBe("/home/user/projects/no");
    // Typing faster than the delay asks once.
    const before = asked.mock.calls.length;
    type("/a");
    type("/ab");
    await new Promise((resolve) => setTimeout(resolve, LIST_DELAY_MS * 2));
    expect(asked.mock.calls.length).toBe(before + 1);
  } finally {
    asked.mockRestore();
  }
});

test("hidden folders when asked, else for a dotted name; a Claude folder is told", async () => {
  const asked = spyOn(transport, "listDirs").mockResolvedValue();
  try {
    render(<Field />);
    await waitFor(() => expect(asked).toHaveBeenCalled());
    answer(asked);
    expect(folders()).not.toContain(".config");
    type("/home/user/.c");
    await waitFor(() => expect(asked).toHaveBeenLastCalledWith("/home/user/.c", false, false));
    answer(asked);
    expect(folders()).toEqual(["/home/", ".claude-work", ".config"]);
    fireEvent.click(screen.getByRole("button", { name: ".claude-work" }));
    expect(screen.getByRole("status").textContent).toBe("");
    type(MOCK_CLAUDE_DIR);
    await waitFor(() => expect(asked).toHaveBeenLastCalledWith(MOCK_CLAUDE_DIR, false, false));
    answer(asked);
    expect(screen.getByRole("status").textContent).toBe("claude");
    cleanup();

    render(<Field hidden />);
    await waitFor(() => expect(asked).toHaveBeenLastCalledWith("", false, true));
    answer(asked);
    expect(folders()).toContain(".config");
  } finally {
    asked.mockRestore();
  }
});

test("`home` fills the empty field with the home folder once listed; blur is passed on", async () => {
  const asked = spyOn(transport, "listDirs").mockResolvedValue();
  let blurred = 0;
  try {
    render(<Field home onBlur={() => blurred++} />);
    await waitFor(() => expect(asked).toHaveBeenCalled());
    answer(asked);
    await waitFor(() => expect(field().value).toBe("/home/user/"));
    fireEvent.blur(field());
    expect(blurred).toBe(1);
    // Without WSL there is no side to pick.
    expect(screen.queryByRole("combobox", { name: "Folder kind" })).toBeNull();
  } finally {
    asked.mockRestore();
  }
});
