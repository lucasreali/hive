import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { apply } from "../reduce";
import { initialState, openModal, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_ACCOUNT_FOLDERS, mockUninstall } from "../transport/mock";
import { paletteCommands } from "./Palette";
import { KIND_LABEL, onUninstalled } from "./UninstallDialog";

// 23.5: the dialog shows the service's list, its two boxes and the confirm; the service removes.

let list: ReturnType<typeof spyOn<typeof transport, "listUninstall">>;
let uninstall: ReturnType<typeof spyOn<typeof transport, "uninstall">>;

beforeEach(() => {
  list = spyOn(transport, "listUninstall").mockResolvedValue();
  uninstall = spyOn(transport, "uninstall").mockResolvedValue();
});

afterEach(() => {
  cleanup();
  list.mockRestore();
  uninstall.mockRestore();
  useHive.setState(initialState, true);
  delete document.documentElement.dataset.closed;
});

const closed = () => document.documentElement.dataset.closed !== undefined;
const dialog = () => screen.getByRole("dialog") as HTMLDialogElement;
const box = (name: string | RegExp) => screen.getByRole("checkbox", { name }) as HTMLInputElement;
const keep = () => box("Keep my settings, spaces and accounts");
const accounts = () => box(/Also remove the Claude account folders/);
const confirm = () => screen.getByRole("button", { name: /^Uninstall/ }) as HTMLButtonElement;
const listed = (keepData: boolean, withAccounts: boolean) =>
  act(() => apply({ type: "uninstall_list", ...mockUninstall(keepData, withAccounts) }));

function open() {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  act(() => openModal("uninstall"));
}

test("the dialog lists exactly what the service says goes, grouped, once it answers", () => {
  // A list from an earlier opening is not shown.
  act(() => apply({ type: "uninstall_list", ...mockUninstall(false, false) }));
  open();
  expect(list).toHaveBeenLastCalledWith(false, false);
  expect(dialog().textContent).toContain("Listing…");
  expect(confirm().disabled).toBe(true);
  listed(false, false);
  const expected = mockUninstall(false, false).items;
  const paths = [...dialog().querySelectorAll("li code")].map((c) => c.textContent);
  expect(paths).toEqual(expected.map((i) => i.path));
  for (const kind of ["data", "program", "cache", "app"] as const) {
    const section = screen.getByRole("region", { name: KIND_LABEL[kind] });
    const want = expected.filter((i) => i.kind === kind).map((i) => i.path);
    expect([...section.querySelectorAll("code")].map((c) => c.textContent)).toEqual(want);
  }
  expect(screen.queryByRole("region", { name: KIND_LABEL.account })).toBeNull();
  expect(keep().checked).toBe(false);
  expect(accounts().checked).toBe(false);
  expect(dialog().textContent).toContain(MOCK_ACCOUNT_FOLDERS[0] as string);
  expect(dialog().textContent).toContain("run /logout in claude first");
  expect(dialog().textContent).toContain("Never touched: ~/.claude, ~/.claude.json");
  // No terminal open: no warning.
  expect(dialog().textContent).not.toContain("uninstalling closes them");
  expect(confirm().disabled).toBe(false);
});

test("each box asks the service again, and only its own answer is shown", () => {
  open();
  listed(false, false);
  fireEvent.click(keep());
  expect(list).toHaveBeenLastCalledWith(true, false);
  // Still the answer for the boxes before: not shown.
  expect(dialog().textContent).toContain("Listing…");
  expect(accounts().disabled).toBe(true);
  listed(true, false);
  expect(screen.queryByRole("region", { name: KIND_LABEL.data })).toBeNull();
  fireEvent.click(keep());
  fireEvent.click(accounts());
  expect(list).toHaveBeenLastCalledWith(false, true);
  listed(false, true);
  const section = screen.getByRole("region", { name: KIND_LABEL.account });
  expect(within(section).getByText(MOCK_ACCOUNT_FOLDERS[0] as string)).toBeTruthy();
  // Keeping the data keeps the account folders too, whatever their box said.
  fireEvent.click(keep());
  expect(list).toHaveBeenLastCalledWith(true, false);
  expect(accounts().checked).toBe(false);
});

test("without account folders Hive made there is no box for them", () => {
  open();
  act(() => apply({ type: "uninstall_list", ...mockUninstall(false, false), account_folders: [] }));
  expect(screen.queryByRole("checkbox", { name: /account folders/ })).toBeNull();
});

test("nothing left to remove cannot be confirmed", () => {
  open();
  act(() => apply({ type: "uninstall_list", ...mockUninstall(false, false), items: [] }));
  expect(dialog().textContent).toContain("Nothing of Hive's is left to remove.");
  expect(confirm().disabled).toBe(true);
});

test("open terminals and agents are said to close, as quitting does", () => {
  open();
  listed(false, false);
  const terminals = (n: number) =>
    Object.fromEntries(Array.from({ length: n }, (_, i) => [i + 1, { channel: i + 1 }]));
  act(() => useHive.setState({ terminals: terminals(1) as never, agents: {} }));
  expect(dialog().textContent).toContain(
    "1 terminal is open: uninstalling closes them, as quitting does.",
  );
  act(() =>
    apply({
      type: "agent_detected",
      channel: 1,
      id: "a",
      project: null,
      worktree: null,
      cwd: null,
    }),
  );
  act(() => useHive.setState({ terminals: terminals(2) as never }));
  expect(dialog().textContent).toContain("2 terminals are open, with 1 agent: uninstalling");
  act(() =>
    apply({
      type: "agent_detected",
      channel: 2,
      id: "b",
      project: null,
      worktree: null,
      cwd: null,
    }),
  );
  expect(dialog().textContent).toContain("with 2 agents");
});

test("confirming sends the choices once, and nothing can be changed or cancelled meanwhile", () => {
  open();
  listed(false, false);
  fireEvent.click(accounts());
  listed(false, true);
  fireEvent.click(confirm());
  expect(uninstall).toHaveBeenCalledTimes(1);
  expect(uninstall).toHaveBeenLastCalledWith(false, true);
  expect(confirm().textContent).toBe("Uninstalling…");
  expect(confirm().disabled).toBe(true);
  expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);
  expect(keep().disabled).toBe(true);
  expect(accounts().disabled).toBe(true);
  expect(screen.queryByTitle("Close (Esc)")).toBeNull();
  // Esc does not close it either.
  const esc = new Event("cancel", { cancelable: true });
  act(() => void dialog().dispatchEvent(esc));
  expect(esc.defaultPrevented).toBe(true);
  expect(useHive.getState().modal).toBe("uninstall");
});

test("Esc and Cancel close the dialog before anything is sent", () => {
  open();
  listed(false, false);
  const esc = new Event("cancel", { cancelable: true });
  act(() => void dialog().dispatchEvent(esc));
  expect(esc.defaultPrevented).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  expect(uninstall).not.toHaveBeenCalled();
});

test("uninstalled, the app quits; what could not go is listed first", () => {
  onUninstalled([]);
  expect(closed()).toBe(true);
  delete document.documentElement.dataset.closed;
  onUninstalled([{ path: "/Applications/Hive.app", error: "denied" }]);
  expect(closed()).toBe(false);
  open();
  listed(false, false);
  fireEvent.click(confirm());
  act(() =>
    apply({ type: "uninstalled", failed: [{ path: "/Applications/Hive.app", error: "denied" }] }),
  );
  expect(screen.getByRole("heading", { name: "Hive is uninstalled" })).toBeTruthy();
  const failed = screen.getByRole("list", { name: "Not removed" });
  expect(failed.textContent).toBe("/Applications/Hive.appdenied");
  fireEvent.click(screen.getByRole("button", { name: "Quit" }));
  expect(closed()).toBe(true);
});

test("the palette offers it on macOS only, as the app menu does", () => {
  const labels = () => paletteCommands(useHive.getState()).map((c) => c.label);
  expect(labels()).not.toContain("Uninstall Hive…");
  const mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
  Object.defineProperty(navigator, "userAgent", { value: mac, configurable: true });
  try {
    const item = paletteCommands(useHive.getState()).find((c) => c.label === "Uninstall Hive…");
    item?.run();
    expect(useHive.getState().modal).toBe("uninstall");
  } finally {
    Reflect.deleteProperty(navigator, "userAgent");
  }
});
