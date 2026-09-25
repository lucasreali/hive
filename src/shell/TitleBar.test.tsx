import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const button = () => screen.queryByTitle("Restart Hive to finish the update");

function offered(agents: string[] = []) {
  const install = spyOn(transport, "installUpdate").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  for (const id of agents) {
    act(() => {
      apply({ type: "agent_detected", channel: 1, id, project: null, worktree: null, cwd: null });
      apply({ type: "agent_state", id, ...agentStatus("working"), subagents: [] });
    });
  }
  act(() => apply({ type: "update_ready", version: "0.2.0" }));
  return install;
}

test("no update, no button", () => {
  render(<App />);
  expect(button()).toBeNull();
});

test("the update button installs at once when no agent would end", () => {
  const install = offered();
  expect(button()?.textContent).toBe("Restart to update to v0.2.0");
  fireEvent.click(button() as HTMLElement);
  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button()?.textContent).toBe("Restarting…");
  expect((button() as HTMLButtonElement).disabled).toBe(true);

  // A failure says why and lets the update be tried again.
  act(() => apply({ type: "update_failed", error: "signature mismatch" }));
  expect(useHive.getState().notice).toBe("Update failed: signature mismatch");
  expect((button() as HTMLButtonElement).disabled).toBe(false);
  install.mockRestore();
});

test("with agents running the update asks first, like closing", () => {
  const install = offered(["a"]);
  fireEvent.click(button() as HTMLElement);
  const dialog = screen.getByRole("dialog", { name: "Update Hive?" });
  expect(dialog.textContent).toContain("1 agent is running. Updating restarts Hive, which ends");
  expect(install).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Update and restart Enter" }));
  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button()?.textContent).toBe("Restarting…");
  install.mockRestore();
});

test("an update failure without an offered update only says why", () => {
  apply({ type: "update_failed", error: "no update to install" });
  expect(useHive.getState()).toMatchObject({
    update: null,
    notice: "Update failed: no update to install",
  });
});
