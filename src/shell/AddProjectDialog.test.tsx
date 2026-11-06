import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { asMac } from "../../test/mac";
import { notice } from "../../test/notice";
import { LINK_DOWN, unsent } from "../../test/unsent";
import { App } from "../App";
import { apply } from "../reduce";
import { initialState, openModal, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS, MOCK_SHARE, MOCK_SHARE_WARNING } from "../transport/mock";
import { savedParent, savedWindows } from "./AddProjectDialog";

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
const kind = () => screen.queryByRole("combobox", { name: "Folder kind" });
/** Picks a side in the Folder kind select, as a user does: open it, click the option. */
const pick = (side: string) => {
  fireEvent.mouseDown(kind() as HTMLElement);
  fireEvent.click(screen.getByRole("option", { name: side }));
};
const submit = () => screen.getByRole("button", { name: "Add project" }) as HTMLButtonElement;
const folders = () =>
  [...screen.getByRole("list", { name: "Folders" }).querySelectorAll("li")].map(
    (li) => li.textContent,
  );
const type = (value: string) => fireEvent.change(field(), { target: { value } });

test("the field's example is a home folder of the platform (18.25)", () => {
  open();
  expect(field().placeholder).toBe("/home/you/projects/shop");
  cleanup();
  asMac();
  render(<App />);
  act(() => openModal("add-project"));
  expect(field().placeholder).toBe("/Users/you/projects/shop");
});

test("the dialog opens focused on the field, which shows the home folder once listed", async () => {
  const dialog = open();
  expect(dialog.open).toBe(true);
  expect(document.activeElement).toBe(field());
  expect(submit().disabled).toBe(true);
  // Without WSL (macOS) there is no Windows side to pick.
  expect(kind()).toBeNull();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  expect(folders()).toEqual(["/home/", "dotfiles", "Downloads", "projects", "work"]);
  expect(submit().disabled).toBe(false);
  expect(dialog.textContent).toContain("A git repository, or any folder inside one");
  expect(dialog.textContent).not.toContain("Windows folders are slower");
});

test("a folder on a network volume shows the service's warning and can still be added", async () => {
  open();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  type(`${MOCK_SHARE}/`);
  await waitFor(() => expect(screen.getByText(MOCK_SHARE_WARNING)).toBeDefined());
  expect(screen.getByText(MOCK_SHARE_WARNING).className).toBe("field-warning");
  expect(submit().disabled).toBe(false);
  // Gone once the field leaves it.
  type("/home/user/");
  await waitFor(() => expect(screen.queryByText(MOCK_SHARE_WARNING)).toBeNull());
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

test("an add that cannot be sent is shown as an error toast; the dialog stays usable (9.21)", async () => {
  const dialog = open();
  type(shop.path);
  await waitFor(() => expect(submit().disabled).toBe(false));
  const restore = unsent("addProject");
  fireEvent.click(submit());
  await waitFor(() => expect(notice()).toBe(LINK_DOWN));
  restore();
  expect(
    within(screen.getByRole("status", { name: "Messages" })).getByText(LINK_DOWN),
  ).toBeDefined();
  expect(dialog.open).toBe(true);
  expect(submit().disabled).toBe(false);
  expect(field().value).toBe(shop.path);
});

test("in WSL, Windows folders are browsed as Windows paths and the choice is remembered", async () => {
  const add = spyOn(transport, "addProject");
  const dialog = open("Ubuntu");
  expect(kind()?.textContent).toBe("WSL");
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  pick("Windows");
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
  expect(kind()?.textContent).toBe("Windows");
  await waitFor(() => expect(field().value).toBe("C:\\Users\\user\\"));
  pick("WSL");
  expect(localStorage.getItem("hive.folderSide")).toBe("wsl");
  await waitFor(() => expect(field().value).toBe("/home/user/"));
});

test("with the service on Windows, every path is a Windows one, drives included", async () => {
  const add = spyOn(transport, "addProject");
  // The mock service runs natively too, so it answers Windows paths.
  await transport.setMode("native");
  await new Promise((resolve) => setTimeout(resolve, 0));
  try {
    open();
    act(() => apply({ type: "app_mode", mode: "native", wsl: false }));
    act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
    // No WSL side to pick: the service's own paths are Windows ones.
    expect(kind()).toBeNull();
    expect(field().placeholder).toBe("C:\\Users\\you\\projects\\shop");
    await waitFor(() => expect(field().value).toBe("C:\\Users\\user\\"));
    expect(folders()).toEqual(["C:\\Users\\", "Documents", "source"]);
    fireEvent.click(screen.getByRole("button", { name: "source" }));
    expect(field().value).toBe("C:\\Users\\user\\source\\");
    // The text after the last `\` filters.
    type("C:\\Users\\user\\D");
    await waitFor(() => expect(folders()).toEqual(["C:\\Users\\", "Documents"]));
    // A name without a separator lists the drives; `C:\` has no parent.
    type("c");
    await waitFor(() => expect(folders()).toEqual(["C:"]));
    fireEvent.click(screen.getByRole("button", { name: "C:" }));
    expect(field().value).toBe("C:\\");
    await waitFor(() => expect(folders()).toEqual(["Users"]));
    type("C:\\Users\\user\\source\\site");
    await waitFor(() => expect(submit().disabled).toBe(false));
    fireEvent.click(submit());
    expect(add).toHaveBeenCalledWith("C:\\Users\\user\\source\\site");
  } finally {
    add.mockRestore();
    cleanup();
    await transport.setMode("wsl");
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
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
    expect(kind()?.textContent).toBe("WSL");
    pick("Windows");
    expect(kind()?.textContent).toBe("Windows");
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
    expect(dialog.querySelector("button kbd")).toBeNull();
    if (how === "Escape") fireEvent(dialog, new Event("close"));
    else if (how === "Cancel") fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    else fireEvent.click(screen.getByTitle(how));
    expect(useHive.getState().modal).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    cleanup();
  }
});

test("the dialog stays closed while the connection is blocked", () => {
  open();
  act(() => apply({ type: "disconnected", reason: "gone", bundled: false }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

// "New project" (23.2).
const toNew = () => fireEvent.click(screen.getByRole("button", { name: "New project" }));
const parentField = () => screen.getByLabelText("Parent folder") as HTMLInputElement;
const nameField = () => screen.getByLabelText("Name") as HTMLInputElement;
const create = () => screen.getByRole("button", { name: "Create project" }) as HTMLButtonElement;
const box = (name: string) => screen.getByRole("checkbox", { name }) as HTMLInputElement;
const named = (value: string) => fireEvent.change(nameField(), { target: { value } });

test("New project asks for a parent folder, a name, git and claude", async () => {
  open();
  const existing = screen.getByRole("button", { name: "Existing folder" });
  expect(existing.getAttribute("aria-pressed")).toBe("true");
  toNew();
  const fresh = screen.getByRole("button", { name: "New project" });
  expect(fresh.getAttribute("aria-pressed")).toBe("true");
  // The parent starts at the home folder when no project was made yet.
  await waitFor(() => expect(parentField().value).toBe("/home/user/"));
  expect(box("Initialize git").checked).toBe(true);
  expect(box("Open a terminal running claude").checked).toBe(true);
  expect(screen.queryByText(/A git repository, or any folder inside one/)).toBeNull();
  // A name is needed.
  expect(create().disabled).toBe(true);
  named("shop-two");
  expect(create().disabled).toBe(false);
  // Back to an existing folder.
  fireEvent.click(existing);
  expect(screen.getByLabelText("Folder")).toBeDefined();
  await waitFor(() => expect(submit().disabled).toBe(false));
});

test("a new project is made in the parent, selected, and opens a terminal running claude", async () => {
  const createProject = spyOn(transport, "createProject");
  const openTerminal = spyOn(transport, "openTerminal");
  const write = spyOn(transport, "writeTerminal");
  open();
  toNew();
  await waitFor(() => expect(parentField().value).toBe("/home/user/"));
  fireEvent.change(parentField(), { target: { value: "/home/user/projects/" } });
  named("market");
  await waitFor(() => expect(create().disabled).toBe(false));
  fireEvent.click(create());
  expect(createProject).toHaveBeenCalledWith("/home/user/projects/", "market", true);
  const path = "/home/user/projects/market";
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  expect(useHive.getState().projects?.[path]?.folder).toBeUndefined();
  expect(useHive.getState().selection).toBe(path);
  expect(openTerminal.mock.calls[0]?.slice(0, 3)).toEqual([path, 80, 24]);
  const id = useHive.getState().tabs[0]?.id as number;
  await waitFor(() => expect(write).toHaveBeenCalledWith(id, "claude\r"));
  closeTerminal(id);
  // The next one starts in the same parent.
  act(() => openModal("add-project"));
  toNew();
  expect(parentField().value).toBe("/home/user/projects/");
  createProject.mockRestore();
  openTerminal.mockRestore();
  write.mockRestore();
});

test("without git it is a folder project, and without the box no terminal opens", async () => {
  const createProject = spyOn(transport, "createProject");
  const openTerminal = spyOn(transport, "openTerminal");
  open();
  toNew();
  await waitFor(() => expect(parentField().value).toBe("/home/user/"));
  named("ideas");
  fireEvent.click(box("Initialize git"));
  fireEvent.click(box("Open a terminal running claude"));
  await waitFor(() => expect(create().disabled).toBe(false));
  fireEvent.click(create());
  expect(createProject).toHaveBeenCalledWith("/home/user/", "ideas", false);
  await waitFor(() => expect(useHive.getState().modal).toBeNull());
  expect(useHive.getState().projects?.["/home/user/ideas"]?.folder).toBe(true);
  expect(useHive.getState().selection).toBe("/home/user/ideas");
  expect(openTerminal).not.toHaveBeenCalled();
  createProject.mockRestore();
  openTerminal.mockRestore();
});

test("the service's refusal of a name shows under it until it is edited", async () => {
  open();
  toNew();
  await waitFor(() => expect(parentField().value).toBe("/home/user/"));
  const refused = async (name: string, message: string) => {
    named(name);
    await waitFor(() => expect(create().disabled).toBe(false));
    fireEvent.click(create());
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe(message));
    expect(nameField().getAttribute("aria-invalid")).toBe("true");
    expect(useHive.getState().modal).toBe("add-project");
  };
  const one = "A name is one folder: no /, no \\, no leading dot, no control characters";
  await refused(".hidden", one);
  await refused("projects", "/home/user/projects already exists");
  named("fresh");
  expect(screen.queryByRole("alert")).toBeNull();
  // A parent 23.1 refuses: in a Claude folder.
  fireEvent.change(parentField(), { target: { value: "/home/user/.claude-work/" } });
  const claude = "/home/user/.claude-work/x is in Claude's folder";
  await refused("x", `${claude}: Hive does not follow it; choose a folder in it`);
  fireEvent.change(parentField(), { target: { value: "/home/user/" } });
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a blocked storage only loses the remembered parent", async () => {
  const fail = () => {
    throw new Error("blocked");
  };
  expect(savedParent({ getItem: fail })).toBe("");
  expect(savedParent(null)).toBe("");
  expect(savedParent({ getItem: () => "/p/" })).toBe("/p/");
  const real = Object.getOwnPropertyDescriptor(window, "localStorage") as PropertyDescriptor;
  const blocked = { getItem: () => null, setItem: fail };
  Object.defineProperty(window, "localStorage", { configurable: true, get: () => blocked });
  try {
    open();
    toNew();
    await waitFor(() => expect(parentField().value).toBe("/home/user/"));
    named("blocked");
    fireEvent.click(box("Open a terminal running claude"));
    await waitFor(() => expect(create().disabled).toBe(false));
    fireEvent.click(create());
    await waitFor(() => expect(useHive.getState().modal).toBeNull());
  } finally {
    Object.defineProperty(window, "localStorage", real);
  }
});

test("New folder makes a folder in the typed one and enters it", async () => {
  const createDir = spyOn(transport, "createDir");
  open();
  await waitFor(() => expect(field().value).toBe("/home/user/"));
  const button = () => screen.getByRole("button", { name: "New folder" }) as HTMLButtonElement;
  // Only for a folder typed, not a name being typed.
  type("/home/user/proj");
  await waitFor(() => expect(button().disabled).toBe(true));
  expect(button().title).toBe("Type or pick a folder first");
  type("/home/user/");
  await waitFor(() => expect(button().disabled).toBe(false));
  fireEvent.click(button());
  const input = () => screen.getByLabelText("New folder name") as HTMLInputElement;
  expect(document.activeElement).toBe(input());
  // Esc closes the name, not the dialog.
  fireEvent.keyDown(input(), { key: "Escape" });
  expect(screen.queryByLabelText("New folder name")).toBeNull();
  expect(useHive.getState().modal).toBe("add-project");
  fireEvent.click(button());
  // A taken name is refused under it; editing clears it.
  fireEvent.change(input(), { target: { value: "projects" } });
  fireEvent.keyDown(input(), { key: "Enter" });
  expect(createDir).toHaveBeenCalledWith("/home/user/", "projects");
  const taken = "/home/user/projects already exists";
  await waitFor(() => expect(screen.getByRole("alert").textContent).toBe(taken));
  expect(input().getAttribute("aria-invalid")).toBe("true");
  fireEvent.change(input(), { target: { value: "games" } });
  expect(screen.queryByRole("alert")).toBeNull();
  fireEvent.keyDown(input(), { key: "a" });
  fireEvent.keyDown(input(), { key: "Enter" });
  await waitFor(() => expect(field().value).toBe("/home/user/games/"));
  expect(screen.queryByLabelText("New folder name")).toBeNull();
  expect(document.activeElement).toBe(field());
  await waitFor(() => expect(folders()).toEqual(["/home/user/", "No folders here"]));
  // Listed in its folder from now on.
  type("/home/user/");
  await waitFor(() => expect(folders()).toContain("games"));
  createDir.mockRestore();
});
