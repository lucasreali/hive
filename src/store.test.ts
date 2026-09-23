import { beforeEach, expect, test } from "bun:test";
import { act, renderHook } from "@testing-library/react";
import {
  apply,
  initialState,
  openModal,
  type ServiceMessage,
  select,
  setRightPanel,
  toggleCollapsed,
  useAgent,
  useHive,
  useTerminal,
} from "./store";
import { MOCK_REPOS } from "./transport/mock";

beforeEach(() => useHive.setState(initialState, true));

test("starts connecting with no data", () => {
  const s = useHive.getState();
  expect(s.connection).toEqual({ status: "connecting" });
  expect(s.terminals).toEqual({});
  expect(s.modal).toBeNull();
});

test("welcome and version_mismatch set the connection", () => {
  apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" });
  expect(useHive.getState().connection).toEqual({
    status: "connected",
    version: "0.1.0",
    distro: "Ubuntu",
  });
  const versions = { protocol: 2, version: "0.2.0", app_protocol: 1, app_version: "0.1.0" };
  apply({ type: "version_mismatch", ...versions });
  expect(useHive.getState().connection).toEqual({ status: "version_mismatch", ...versions });
});

test("disconnected keeps the reason; unknown messages change nothing", () => {
  apply({ type: "disconnected", reason: "the hive bridge exited" });
  const before = useHive.getState();
  expect(before.connection).toEqual({ status: "disconnected", reason: "the hive bridge exited" });
  apply({ type: "agent" } as unknown as ServiceMessage);
  expect(useHive.getState()).toEqual(before);
});

test("terminal messages update only their terminal", () => {
  apply({ type: "terminal_opened", channel: 1 });
  apply({ type: "terminal_opened", channel: 2 });
  apply({ type: "unhooked_agent", channel: 1 });
  apply({ type: "terminal_exited", channel: 1, code: 3 });
  const { terminals } = useHive.getState();
  expect(terminals[1]).toEqual({ id: 1, exited: true, code: 3, unhooked: true });
  expect(terminals[2]).toEqual({ id: 2, exited: false, code: null, unhooked: false });
  apply({ type: "terminal_opened", channel: 1 });
  expect(useHive.getState().terminals[1]).toEqual({
    id: 1,
    exited: false,
    code: null,
    unhooked: false,
  });
});

test("ui actions set ui state", () => {
  openModal("new-worktree");
  setRightPanel("files");
  select("wt-1");
  const s = useHive.getState();
  expect([s.modal, s.rightPanel, s.selection]).toEqual(["new-worktree", "files", "wt-1"]);
});

test("useAgent re-renders only when its own agent changes", () => {
  useHive.setState({ agents: { a: { id: "a" }, b: { id: "b" } } });
  let renders = 0;
  const { result } = renderHook(() => {
    renders++;
    return useAgent("a");
  });
  expect(result.current).toEqual({ id: "a" });
  act(() => useHive.setState((s) => ({ agents: { ...s.agents, b: { id: "b" } } })));
  expect(renders).toBe(1);
  act(() => useHive.setState((s) => ({ agents: { ...s.agents, a: { id: "a" } } })));
  expect(renders).toBe(2);
});

test("useTerminal reads one terminal", () => {
  apply({ type: "terminal_opened", channel: 4 });
  const { result } = renderHook(() => useTerminal(4));
  expect(result.current?.id).toBe(4);
});

test("projects replace the list; an added project joins it and closes its dialog", () => {
  const [shop, api] = MOCK_REPOS;
  expect(useHive.getState().projects).toBeNull();
  apply({ type: "projects", projects: [shop] });
  expect(useHive.getState().projects).toEqual({ [shop.id]: shop });
  openModal("add-project");
  apply({ type: "add_project_failed", path: "x", error: "not_absolute", message: "m" });
  expect(useHive.getState().addProjectError).toBe("m");
  apply({ type: "project_added", project: api });
  const s = useHive.getState();
  expect(Object.keys(s.projects ?? {})).toEqual([shop.id, api.id]);
  expect([s.modal, s.addProjectError]).toEqual([null, null]);
  // Another dialog stays open.
  openModal("new-worktree");
  apply({ type: "project_added", project: shop });
  expect(useHive.getState().modal).toBe("new-worktree");
  apply({ type: "projects", projects: [] });
  expect(useHive.getState().projects).toEqual({});
});

test("tree nodes toggle between collapsed and expanded", () => {
  toggleCollapsed("p");
  expect(useHive.getState().collapsed).toEqual({ p: true });
  toggleCollapsed("p");
  expect(useHive.getState().collapsed).toEqual({ p: false });
});
