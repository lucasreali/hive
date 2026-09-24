import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, openModal, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { savedWindows } from "./AddProjectDialog";

beforeAll(async () => {
  // The mock service answers listings and adds.
  await transport.connect(apply);
  await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
  localStorage.clear();
});

const [shop] = MOCK_REPOS;
const site = MOCK_REPOS[3] as (typeof MOCK_REPOS)[number];

/** Opens the dialog; `distro` is null when the service does not run in WSL (macOS). */
function open(distro: string | null = null) {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro }));
  act(() => apply({ type: "projects", projects: [] }));
  fireEvent.click(screen.getByRole("button", { name: "Add project Ctrl+Shift+O" }));
  return screen.getByRole("dialog", { name: "Add project" }) as HTMLDialogElement;
}

const field = () => screen.getByLabelText("Folder") as HTMLInputElement;
const kind = () => screen.queryByLabelText("Folder kind") as HTMLSelectElement | null;
const submit = () => screen.getByRole("button", { name: "Add project Enter" }) as HTMLButtonElement;
const folders = () =>
  [...screen.getByRole("list", { name: "Folders" }).querySelectorAll("li")].map(
    (li) => li.textContent,
  );
const type = (value: string) => fireEvent.change(field(), { target: { value } });

test("the dialog opens focused on the field, which shows the home folder once listed", async () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  expect(document.activeElement).toBe(field());
  expect(submit().disabled).toBe(true);
  // Without WSL (macOS) there is no Windows side to pick.
  expect(kind()).toBeNull();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  expect(folders()).toEqual(["/home/", "dotfiles", "Downloads", "projects"]);
  expect(submit().disabled).toBe(false);
  expect(dialog.textContent).toContain("A git repository, or any folder inside one");
  expect(dialog.textContent).not.toContain("Windows folders are slower");
});

test("typing filters the typed folder's subfolders; a click enters one and ↑ goes up", async () => {
  open();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  type("/home/user/projects/S");
  await waitFor(() => expect(folders()).toEqual(["/home/user/", "notes", "shop"]));
  // Repositories carry a git icon.
  expect(screen.getByLabelText("Repository")).toBeDefined();
  fireEvent.click(screen.getByRole("button", { name: "Repository shop" }));
  expect(field().value).toBe("/home/user/projects/shop/");
  expect(document.activeElement).toBe(field());
  await waitFor(() => expect(folders()).toEqual(["/home/user/projects/", "No folders here"]));
  fireEvent.click(screen.getByTitle("Up a level"));
  expect(field().value).toBe("/home/user/projects/");
  await waitFor(() => expect(folders()).toEqual(["/home/user/", "api", "notes", "shop"]));
  // A folder that does not exist says so.
  type("/nowhere/x");
  await waitFor(() =>
    expect(folders()).toEqual([expect.stringContaining("cannot open /nowhere/")]),
  );
  expect(submit().disabled).toBe(true);
});

test("a cleared field lists the home folder without filling it again", async () => {
  open();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  type("/home/user/projects/");
  await waitFor(() => expect(folders()).toContain("api"));
  type("");
  await waitFor(() => expect(folders()).toContain("dotfiles"));
  expect(field().value).toBe("");
  fireEvent.click(screen.getByRole("button", { name: "Repository dotfiles" }));
  expect(field().value).toBe("/home/user/dotfiles/");
});

test("the typed path's Linux form goes to the service, and the answer closes the dialog", async () => {
  const add = spyOn(transport, "addProject");
  open();
  type(shop.path);
  await waitFor(() => expect(submit().disabled).toBe(false));
  fireEvent.click(submit());
  expect(add).toHaveBeenCalledWith(shop.path);
  add.mockRestore();
  // Tests run outside Tauri, so the mock service answers.
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByRole("button", { name: "shop" })).toBeDefined();
});

test("in WSL, Windows folders are browsed as Windows paths and the choice is remembered", async () => {
  const add = spyOn(transport, "addProject");
  const dialog = open("Ubuntu");
  expect(kind()?.value).toBe("wsl");
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  fireEvent.change(kind() as HTMLSelectElement, { target: { value: "windows" } });
  expect(localStorage.getItem("hive.folderSide")).toBe("windows");
  expect(field().placeholder).toBe("C:\\Users\\you\\projects\\shop");
  await waitFor(() => expect(field().value).toBe("C:\\Users\\user\\"));
  expect(folders()).toEqual(["C:\\Users\\", "Documents", "source"]);
  expect(dialog.textContent).toContain(
    "Windows folders are slower and don't update live; WSL folders are recommended.",
  );
  fireEvent.click(screen.getByRole("button", { name: "source" }));
  expect(field().value).toBe("C:\\Users\\user\\source\\");
  await waitFor(() => expect(folders()).toContain("site"));
  type("C:\\Users\\user\\source\\site");
  await waitFor(() => expect(submit().disabled).toBe(false));
  fireEvent.click(submit());
  expect(add).toHaveBeenCalledWith(site.path);
  add.mockRestore();
  await waitFor(() => expect(useHive.getState().modal).toBeNull());

  // The next time, the dialog starts on the Windows side.
  cleanup();
  useHive.setState(initialState, true);
  open("Ubuntu");
  expect(kind()?.value).toBe("windows");
  await waitFor(() => expect(field().value).toBe("C:\\Users\\user\\"));
  fireEvent.change(kind() as HTMLSelectElement, { target: { value: "wsl" } });
  expect(localStorage.getItem("hive.folderSide")).toBe("wsl");
  await waitFor(() => expect(field().value).toBe("/home/user/"));
});

test("a blocked storage only loses the remembered choice", async () => {
  const fail = () => {
    throw new Error("blocked");
  };
  expect(savedWindows({ getItem: fail })).toBe(false);
  expect(savedWindows(null)).toBe(false);
  const real = Object.getOwnPropertyDescriptor(window, "localStorage") as PropertyDescriptor;
  let writes = 0;
  const blocked = {
    getItem: () => null,
    setItem: () => {
      writes++;
      fail();
    },
  };
  Object.defineProperty(window, "localStorage", { configurable: true, get: () => blocked });
  try {
    open("Ubuntu");
    expect(kind()?.value).toBe("wsl");
    fireEvent.change(kind() as HTMLSelectElement, { target: { value: "windows" } });
    expect(kind()?.value).toBe("windows");
    expect(writes).toBe(1);
  } finally {
    Object.defineProperty(window, "localStorage", real);
  }
});

test("editing the field clears the refusal", () => {
  open();
  type("/");
  act(() =>
    apply({ type: "add_project_failed", path: "/", error: "not_a_git_repository", message: "m" }),
  );
  type("");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(field().getAttribute("aria-invalid")).toBe("false");
});

test("a refused path is explained under the field until the dialog reopens", () => {
  open();
  const message = "cannot open /x: No such file or directory (os error 2)";
  act(() => apply({ type: "add_project_failed", path: "/x", error: "not_found", message }));
  expect(screen.getByRole("alert").textContent).toBe(message);
  expect(field().getAttribute("aria-invalid")).toBe("true");
  act(() => openModal("add-project"));
  expect(screen.queryByRole("alert")).toBeNull();
});

test("close, cancel and Esc close the dialog", () => {
  for (const how of ["Close (Esc)", "Cancel", "Escape"]) {
    const dialog = open();
    if (how === "Escape") fireEvent(dialog, new Event("close"));
    else if (how === "Cancel") fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" }));
    else fireEvent.click(screen.getByTitle(how));
    expect(useHive.getState().modal).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    cleanup();
  }
});

test("the dialog stays closed while the connection is blocked", () => {
  open();
  act(() => apply({ type: "disconnected", reason: "gone" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
