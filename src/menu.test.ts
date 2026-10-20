import { afterAll, afterEach, expect, spyOn, test } from "bun:test";
import { connect } from "./connect";
import { HELP_URL, ISSUE_URL, MENU_ITEMS, runMenuItem } from "./menu";
import { apply } from "./reduce";
import { COMMANDS } from "./shortcuts";
import { initialState, openModal, useHive } from "./store";
import { transport } from "./transport";
import { MOCK_REPOS } from "./transport/mock";

// 17.13: the app menu's own items reach the UI as `menu {item}`; the mock transport sends it for
// `menu <item>` typed in a terminal.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 10; i++) await tick();
};
const notices = () => useHive.getState().notices.map((n) => `${n.kind}:${n.text}`);
const lastNotice = () => notices().at(-1);

afterEach(() => {
  openModal(null);
  for (const dialog of document.querySelectorAll("dialog")) dialog.remove();
  useHive.setState({ notices: [], sidebarShown: true, rightPanel: "files" });
});
afterAll(() => useHive.setState(initialState, true));

test("the UI knows every item of the app side's menu but Quit, which the app side answers", () => {
  // As `src-tauri/src/menu.rs`'s `MENUS` (its test lists the same ids).
  expect([...MENU_ITEMS].sort()).toEqual(
    [
      "settings",
      "check-updates",
      "toggle-sidebar",
      "toggle-panel",
      "close-tab",
      "help",
      "report-issue",
    ].sort(),
  );
});

test("a menu item from the app side runs its action", async () => {
  await connect();
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const shell = await transport.openTerminal(shop.path, 80, 24, () => {});
  const click = async (item: string) => {
    await transport.writeTerminal(shell, `menu ${item}\r`);
    await settle();
  };
  await click("toggle-sidebar");
  expect(useHive.getState().sidebarShown).toBe(false);
  await click("toggle-sidebar");
  expect(useHive.getState().sidebarShown).toBe(true);
  await click("toggle-panel");
  expect(useHive.getState().rightPanel).toBeNull();
  await click("toggle-panel");
  expect(useHive.getState().rightPanel).toBe("files");
  // Not the Hive app: links only say so.
  await click("help");
  expect(lastNotice()).toBe(`error:Only the Hive app opens links: ${HELP_URL}`);
  await click("report-issue");
  expect(lastNotice()).toBe(`error:Only the Hive app opens links: ${ISSUE_URL}`);
  await click("check-updates");
  expect(lastNotice()).toBe("info:Hive is up to date.");
  await click("settings");
  expect(useHive.getState().modal).toBe("settings");
});

test("Close Tab runs ⌘W's command; an unknown item does nothing", () => {
  const close = COMMANDS.find((c) => c.id === "close-tab") as (typeof COMMANDS)[number];
  const run = spyOn(close, "run").mockImplementation(() => {});
  runMenuItem("close-tab");
  expect(run).toHaveBeenCalledTimes(1);
  run.mockRestore();
  const before = useHive.getState();
  for (const item of ["quit", "about", "__proto__", "toString", ""]) runMenuItem(item);
  expect(useHive.getState()).toBe(before);
});

test("with a dialog open only the items that act outside the window run", async () => {
  openModal("palette");
  document.body.append(Object.assign(document.createElement("dialog"), { open: true }));
  runMenuItem("toggle-sidebar");
  runMenuItem("settings");
  expect(useHive.getState()).toMatchObject({ sidebarShown: true, modal: "palette" });
  runMenuItem("check-updates");
  await settle();
  expect(lastNotice()).toBe("info:Hive is up to date.");
});

test("a check asked for says why it failed", () => {
  apply({ type: "update_checked", error: "offline" });
  expect(lastNotice()).toBe("error:Cannot check for updates: offline");
});
