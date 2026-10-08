import { expect, test } from "@playwright/test";

test("states: every agent and subagent shows the state icon the service sent", async ({ page }) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const rows = tree.locator(".tree-row.agent, .subagent-line");
  await expect(rows).toHaveCount(11);
  const shown = await rows.evaluateAll((els) =>
    els.map((el) => {
      const icon = el.querySelector(".state-icon") as SVGElement;
      // The layout box: working spins, so its bounding box grows mid-rotation.
      const box = getComputedStyle(icon);
      return [
        el.className,
        icon.getAttribute("aria-label"),
        el.querySelector(".label, .subagent-type")?.textContent,
        box.width,
        box.height,
      ];
    }),
  );
  expect(shown).toEqual([
    ["tree-row agent", "running subagents", "Claude", "14px", "14px"],
    ["subagent-line", "working", "Explore", "11px", "11px"],
    ["subagent-line", "working", "subagent", "11px", "11px"],
    ["tree-row agent", "waiting for permission", "Claude", "14px", "14px"],
    ["subagent-line", "waiting for permission", "general-purpose", "11px", "11px"],
    ["subagent-line", "waiting for your answer", "Explore", "11px", "11px"],
    ["tree-row agent", "waiting for you", "Claude", "14px", "14px"],
    ["tree-row agent", "error", "Claude", "14px", "14px"],
    ["tree-row agent", "ended", "Claude", "14px", "14px"],
    ["tree-row agent", "working", "Claude", "14px", "14px"],
    ["tree-row agent", "idle", "Claude", "14px", "14px"],
  ]);
  // Colors come from the state tokens; only alerting states color the agent's state name.
  const permission = tree.locator(".tree-row.agent .state-label[data-state=waiting_permission]");
  await expect(permission).toHaveCSS("color", "rgb(222, 193, 132)");
  const working = tree.locator(".tree-row.agent .state-label[data-state=working]");
  await expect(working).toHaveCSS("color", "rgb(169, 175, 188)");
  // Working spins, in its state's color, on a subagent's line too.
  const spinning = tree.locator(".tree-row.agent .state-icon[data-state=working]");
  await expect(spinning).toHaveCSS("animation-name", "hive-spin");
  await expect(spinning).toHaveCSS("color", "rgb(116, 173, 232)");
  const subSpinning = tree.locator(".subagent-line .state-icon[data-state=working]").first();
  await expect(subSpinning).toHaveCSS("color", "rgb(116, 173, 232)");
  // After the state's name: the time in it and what the agent is doing, muted, on one line.
  const meta = permission.locator("xpath=..").locator(".state-meta");
  await expect(meta).toHaveText(/^\d+m · Editing src\/auth\/login\.ts$/);
  await expect(meta).toHaveCSS("white-space", "nowrap");
});

test("states: a subagent is one quiet line under its agent that does nothing when clicked", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const line = tree.locator(".subagent-line").first();
  const agent = tree.locator(".tree-row.agent").first();
  // Its type, what it is doing and the time in its state, on one line shorter than an agent's
  // row, smaller and muted as the agent's time and activity are.
  await expect(line.locator(".subagent-type")).toHaveText("Explore");
  await expect(line.locator(".subagent-activity")).toHaveText("Searching useSession");
  await expect(line.locator(".subagent-activity")).toHaveCSS("text-overflow", "ellipsis");
  await expect(line.locator(".subagent-time")).toHaveText(/^\d+[smh]$/);
  const [lineBox, agentBox] = [await line.boundingBox(), await agent.boundingBox()];
  expect(lineBox?.height).toBe(18);
  expect(agentBox?.height).toBeGreaterThan(18);
  await expect(line).toHaveCSS("font-size", "11px");
  const muted = await agent.locator(".state-meta").evaluate((el) => getComputedStyle(el).color);
  await expect(line).toHaveCSS("color", muted);
  // No hover state, nothing to focus, and a click or a right-click changes nothing.
  const current = page.locator("[aria-current=true]");
  const before = await current.count();
  await line.hover();
  await expect(line).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(line).toHaveCSS("cursor", "default");
  await expect(line.locator("button, [tabindex]")).toHaveCount(0);
  await line.click();
  await line.click({ button: "right" });
  await expect(current).toHaveCount(before);
  await expect(agent).toHaveAttribute("data-selected", "false");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.locator(".file-view")).toHaveCount(0);
});

test("states: collapsed nodes show the most urgent state inside; F8 walks the pending agents", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const chip = page.getByRole("button", { name: "3 pending: notifications" });
  await expect(chip).toBeVisible();
  await expect(chip).toHaveCSS("color", "rgb(222, 193, 132)");

  // Collapsed: the project shows waiting for permission (a subagent's, via its agent).
  await tree.getByRole("button", { name: "Collapse shop" }).click();
  const shop = tree.locator(".tree-row.project").first();
  const icon = shop.locator(".state-icon");
  await expect(icon).toHaveAttribute("data-state", "waiting_permission");
  expect(await icon.boundingBox()).toMatchObject({ width: 14, height: 14 });
  await tree.getByRole("button", { name: "Collapse refactor-auth" }).click();
  const refactor = tree.locator(".tree-row.worktree", { hasText: "refactor-auth" });
  await expect(refactor.locator(".state-icon")).toHaveAttribute("data-state", "working");

  // F8 opens the collapsed project and selects each pending agent in tree order, wrapping.
  const selected = tree.locator(".tree-row.agent[data-selected=true] .state-label");
  const expected = ["waiting for permission", "waiting for you", "error", "waiting for permission"];
  for (const label of expected) {
    await page.keyboard.press("F8");
    await expect(selected).toHaveText(label);
  }
  await expect(icon).toHaveCount(0);
  // The bell's inbox lists the pending agents in tree order; clicking one goes to it.
  await chip.click();
  await page.getByRole("menuitem").nth(1).click();
  await expect(selected).toHaveText("waiting for you");
});

test("states: a subagent's own worktree has no row; its path is the line's tooltip", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  const line = tree.locator(".subagent-line", { hasText: "general-purpose" });
  await expect(line).toHaveAttribute(
    "title",
    "/home/user/projects/shop/.claude/worktrees/tests-login",
  );
  await expect(tree.locator(".tree-row", { hasText: "tests-login" })).toHaveCount(0);
  // At the subagents' indent, with the agent's tree line down to each; the last one stops at its
  // line.
  const lines = await tree
    .locator(".subagent-line")
    .evaluateAll((els) =>
      els.map((el) => [getComputedStyle(el).paddingLeft, getComputedStyle(el, "::before").height]),
    );
  expect(lines).toEqual([
    ["56px", "18px"],
    ["56px", "9px"],
    ["56px", "18px"],
    ["56px", "9px"],
  ]);
});

// 15.6: a 🟠 the service no longer counts as pending (looked at) is drawn muted beside one that
// still is, in both themes; its colors are the muted text tokens.
test("states: a seen and an unseen agent waiting for you side by side, dark and light", async ({
  page,
}) => {
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.locator(".tree-row.agent")).toHaveCount(7);
  // api's first two agents become an unseen and a seen 🟠, as the service would send them.
  await page.evaluate(async () => {
    const url = "/src/reduce.ts";
    const { apply } = await import(/* @vite-ignore */ url);
    const status = {
      type: "agent_state",
      state: "waiting_you",
      urgency: 4,
      interrupted: false,
      alert: null,
      notify: false,
      writing: false,
      subagents: [],
      activity: null,
      since_ms: Date.now() - 60_000,
    };
    apply({ ...status, id: "mock-state-4", pending: true });
    apply({ ...status, id: "mock-state-5", pending: false });
  });
  await expect(tree.locator(".tree-row.agent .state-icon[data-state=waiting_you]")).toHaveCount(3);
  const unseen = tree.locator(".tree-row.agent").nth(3);
  const seen = tree.locator(".tree-row.agent").nth(4);
  await expect(seen.locator(".state-icon")).toHaveAttribute("aria-label", "waiting for you, seen");
  const both = seen.locator("xpath=ancestor::ul[1]");
  const themes = {
    dark: ["rgb(224, 138, 90)", "rgb(220, 224, 229)", "rgb(169, 175, 188)", "rgb(135, 138, 152)"],
    light: ["rgb(173, 110, 37)", "rgb(36, 37, 41)", "rgb(88, 88, 90)", "rgb(126, 128, 134)"],
  };
  for (const [theme, [you, text, text2, text3]] of Object.entries(themes)) {
    await page.evaluate((light) => {
      if (light) document.documentElement.dataset.theme = "one-light";
      else delete document.documentElement.dataset.theme;
    }, theme === "light");
    // Unseen: today's orange icon, highlighted title, the usual state name.
    await expect(unseen.locator(".state-icon")).toHaveCSS("color", you);
    await expect(unseen.locator(".label")).toHaveCSS("color", text);
    await expect(unseen.locator(".state-label")).toHaveCSS("color", text2);
    // Seen: icon and "waiting for you" in the muted text color, the title not highlighted.
    await expect(seen.locator(".state-icon")).toHaveCSS("color", text3);
    await expect(seen.locator(".label")).toHaveCSS("color", text2);
    await expect(seen.locator(".state-label")).toHaveCSS("color", text3);
    await both.screenshot({ path: `target/e2e/seen-waiting-you-${theme}.png` });
  }
  // The seen one is not pending: the bell counts the permission and the two unseen 🟠 only.
  await expect(page.getByRole("button", { name: "3 pending: notifications" })).toBeVisible();
});
