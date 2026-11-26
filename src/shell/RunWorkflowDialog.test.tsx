import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { openRunWorkflow } from "../git";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { MOCK_WORKFLOWS } from "../transport/mockGit";
import { startValues } from "./RunWorkflowDialog";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const [shop] = MOCK_REPOS;
const login = shop.worktrees[1];

function open() {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  act(() => openRunWorkflow(shop.id, login));
  return screen.getByRole("dialog", { name: "Run workflow" });
}

const listed = (workflows = MOCK_WORKFLOWS, error: string | null = null) =>
  act(() => apply({ type: "workflows", project: shop.id, worktree: login.path, workflows, error }));

test("an input starts at its default, a choice at its first option, else empty", () => {
  const [ci, release] = MOCK_WORKFLOWS;
  expect(startValues(undefined)).toEqual({});
  expect(startValues(ci)).toEqual({});
  expect(startValues(release)).toEqual({ level: "patch", dry_run: "true", notes: "" });
  const choice = { ...release.inputs[0], default: null };
  expect(startValues({ ...release, inputs: [choice] })).toEqual({ level: "patch" });
});

test("picks a workflow, its branch and inputs, and runs it", () => {
  const list = spyOn(transport, "listWorkflows").mockResolvedValue();
  const run = spyOn(transport, "runWorkflow").mockResolvedValue();
  const dialog = open();
  expect(list).toHaveBeenCalledWith(shop.id, login.path);
  expect(dialog.textContent).toContain("Reading the workflows…");
  const submit = within(dialog).getByRole("button", { name: "Run" }) as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  // Another worktree's answer is not this dialog's.
  act(() =>
    apply({ type: "workflows", project: shop.id, worktree: "/x", workflows: [], error: null }),
  );
  expect(dialog.textContent).toContain("Reading the workflows…");
  listed();
  const branch = within(dialog).getByLabelText("Branch") as HTMLInputElement;
  expect(branch.value).toBe(login.branch ?? "");
  // CI has no input: Run on the branch as is.
  fireEvent.click(submit);
  expect(run).toHaveBeenLastCalledWith(shop.id, "ci.yml", login.branch ?? "", []);
  expect(submit.textContent).toBe("Starting…");
  act(() => apply({ type: "workflow_failed", project: shop.id, message: "HTTP 422: nope" }));
  expect(within(dialog).getByRole("alert").textContent).toBe("HTTP 422: nope");
  // Release: a choice, a boolean and a text, each from its default.
  fireEvent.mouseDown(within(dialog).getByRole("combobox", { name: "Workflow" }));
  fireEvent.click(screen.getByRole("option", { name: "Release (release.yml)" }));
  expect(within(dialog).getByText("Which part of the version to bump")).toBeDefined();
  fireEvent.mouseDown(within(dialog).getByRole("combobox", { name: "level *" }));
  fireEvent.click(screen.getByRole("option", { name: "minor" }));
  const dry = within(dialog).getByLabelText("dry_run") as HTMLInputElement;
  expect(dry.checked).toBe(true);
  expect(dry.parentElement?.title).toBe("Build without publishing");
  fireEvent.click(dry);
  fireEvent.change(within(dialog).getByLabelText("notes"), { target: { value: "-x" } });
  fireEvent.change(branch, { target: { value: " main " } });
  fireEvent.click(submit);
  expect(run).toHaveBeenLastCalledWith(shop.id, "release.yml", "main", [
    { name: "level", value: "minor" },
    { name: "dry_run", value: "false" },
    { name: "notes", value: "-x" },
  ]);
  act(() =>
    apply({ type: "workflow_started", project: shop.id, branch: "main", run: 7, message: "S" }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useHive.getState().openRun?.run).toBe(7);
  list.mockRestore();
  run.mockRestore();
});

test("a required input left empty, no branch, no workflow or why none, and closing", () => {
  const list = spyOn(transport, "listWorkflows").mockResolvedValue();
  const dialog = open();
  const submit = within(dialog).getByRole("button", { name: "Run" }) as HTMLButtonElement;
  const [, release] = MOCK_WORKFLOWS;
  const needed = { ...release, inputs: [{ ...release.inputs[2], required: true }] };
  listed([needed]);
  expect(within(dialog).getByLabelText("notes *")).toBeDefined();
  expect(submit.disabled).toBe(true);
  fireEvent.change(within(dialog).getByLabelText("notes *"), { target: { value: "n" } });
  expect(submit.disabled).toBe(false);
  fireEvent.change(within(dialog).getByLabelText("Branch"), { target: { value: " " } });
  expect(submit.disabled).toBe(true);
  // A number is typed as text, with a decimal keyboard.
  listed([{ ...release, inputs: [{ ...release.inputs[2], kind: "number" }] }]);
  expect(within(dialog).getByLabelText("notes").getAttribute("inputmode")).toBe("decimal");
  listed([]);
  expect(dialog.textContent).toContain("No workflow of this repository can be run by hand");
  listed([], "fatal: bad object HEAD");
  expect(within(dialog).getByRole("alert").textContent).toBe("fatal: bad object HEAD");
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(useHive.getState().modal).toBeNull();
  act(() => openRunWorkflow(shop.id, login));
  fireEvent.click(within(screen.getByRole("dialog")).getByTitle("Close (Esc)"));
  expect(useHive.getState().modal).toBeNull();
  // A worktree that is gone shows nothing.
  act(() => openRunWorkflow(shop.id, { id: "/gone", path: "/gone" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  list.mockRestore();
});
