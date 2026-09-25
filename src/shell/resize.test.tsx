import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import {
  addTab,
  initialState,
  LIMITS,
  savedWidths,
  setRightPanel,
  setSplit,
  setWidth,
  useHive,
} from "../store";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
  localStorage.clear();
});

const handle = (name: string) => screen.getByRole("separator", { name });

test("widths are remembered between runs, within their limits", () => {
  expect(savedWidths()).toEqual({ sidebarWidth: 264, panelWidth: 380, splitPercent: 50 });
  setWidth("sidebar", 9999);
  setWidth("panel", 1);
  expect(useHive.getState()).toMatchObject({
    sidebarWidth: LIMITS.sidebar.max,
    panelWidth: LIMITS.panel.min,
  });
  expect(savedWidths()).toEqual({ sidebarWidth: 480, panelWidth: 280, splitPercent: 50 });
  localStorage.setItem("hive.widths", JSON.stringify({ sidebarWidth: 300.4, panelWidth: "x" }));
  expect(savedWidths()).toEqual({ sidebarWidth: 300, panelWidth: 380, splitPercent: 50 });
  localStorage.setItem("hive.widths", "not json");
  expect(savedWidths()).toEqual({ sidebarWidth: 264, panelWidth: 380, splitPercent: 50 });
  expect(savedWidths(null)).toEqual({ sidebarWidth: 264, panelWidth: 380, splitPercent: 50 });
});

test("a storage that cannot be used only loses the preference", () => {
  const own = Object.getOwnPropertyDescriptor(window, "localStorage");
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    get() {
      throw new Error("blocked");
    },
  });
  expect(savedWidths()).toEqual({ sidebarWidth: 264, panelWidth: 380, splitPercent: 50 });
  setWidth("sidebar", 320);
  expect(useHive.getState().sidebarWidth).toBe(320);
  if (own) Object.defineProperty(window, "localStorage", own);
  const full = {
    setItem: () => {
      throw new Error("full");
    },
  };
  Object.defineProperty(window, "localStorage", { configurable: true, get: () => full });
  setWidth("sidebar", 330);
  expect(useHive.getState().sidebarWidth).toBe(330);
  if (own) Object.defineProperty(window, "localStorage", own);
});

test("arrow keys move each edge, the right panel's from its left side", () => {
  render(<App />);
  act(() => setRightPanel("files"));
  const left = handle("Resize the sidebar");
  expect(left.getAttribute("aria-valuenow")).toBe("264");
  expect([left.getAttribute("aria-valuemin"), left.getAttribute("aria-valuemax")]).toEqual([
    "200",
    "480",
  ]);
  fireEvent.keyDown(left, { key: "ArrowRight" });
  expect(useHive.getState().sidebarWidth).toBe(280);
  fireEvent.keyDown(left, { key: "ArrowLeft" });
  fireEvent.keyDown(left, { key: "ArrowLeft" });
  expect(useHive.getState().sidebarWidth).toBe(248);
  fireEvent.keyDown(left, { key: "Enter" });
  expect(useHive.getState().sidebarWidth).toBe(248);
  expect((screen.getByRole("navigation", { name: "Projects" }) as HTMLElement).style.width).toBe(
    "248px",
  );
  const right = handle("Resize the side panel");
  fireEvent.keyDown(right, { key: "ArrowLeft" });
  expect(useHive.getState().panelWidth).toBe(396);
  fireEvent.keyDown(right, { key: "ArrowRight" });
  expect(useHive.getState().panelWidth).toBe(380);
});

test("dragging sizes a panel; dragging the right one much too narrow closes it", () => {
  render(<App />);
  act(() => setRightPanel("files"));
  const move = (target: HTMLElement, clientX: number) =>
    target.dispatchEvent(new PointerEvent("pointermove", { clientX, bubbles: true }));
  const left = handle("Resize the sidebar");
  fireEvent.pointerDown(left, { pointerId: 1, clientX: 264 });
  // Only the dragged edge is named, so only it is lit.
  expect(document.body.dataset.resizing).toBe("sidebar");
  move(left, 350);
  expect(useHive.getState().sidebarWidth).toBe(350);
  move(left, 10);
  expect(useHive.getState().sidebarWidth).toBe(200);
  left.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  expect(document.body.dataset.resizing).toBeUndefined();
  move(left, 400);
  expect(useHive.getState().sidebarWidth).toBe(200);

  const right = handle("Resize the side panel");
  fireEvent.pointerDown(right, { pointerId: 2 });
  expect(document.body.dataset.resizing).toBe("panel");
  move(right, window.innerWidth - 500);
  expect(useHive.getState().panelWidth).toBe(500);
  // Narrower than its minimum, it stays at the minimum...
  move(right, window.innerWidth - 250);
  expect(useHive.getState().panelWidth).toBe(280);
  // ...and far past it, it closes, keeping its last width for next time.
  move(right, window.innerWidth - 150);
  expect(useHive.getState().rightPanel).toBeNull();
  expect(useHive.getState().panelWidth).toBe(280);
  expect(document.body.dataset.resizing).toBeUndefined();
  // A cancelled drag ends too.
  fireEvent.pointerDown(left, { pointerId: 3 });
  left.dispatchEvent(new PointerEvent("pointercancel", { bubbles: true }));
  expect(document.body.dataset.resizing).toBeUndefined();
});

test("dragging the split terminals' divider sets the left pane's share, remembered", () => {
  render(<App />);
  act(() => {
    addTab(1, "/w");
    addTab(2, "/w");
    setSplit({ left: 1, right: 2 });
  });
  const divider = handle("Resize the split terminals");
  const area = divider.parentElement as HTMLElement;
  area.getBoundingClientRect = () => ({ left: 100, width: 1000 }) as DOMRect;
  fireEvent.pointerDown(divider, { pointerId: 1 });
  expect(document.body.dataset.resizing).toBe("split");
  const move = (clientX: number) =>
    act(() => {
      divider.dispatchEvent(new PointerEvent("pointermove", { clientX, bubbles: true }));
    });
  move(400);
  expect(useHive.getState().splitPercent).toBe(30);
  move(1050);
  expect(useHive.getState().splitPercent).toBe(LIMITS.split.max);
  act(() => {
    divider.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  });
  expect(savedWidths().splitPercent).toBe(80);
  fireEvent.keyDown(divider, { key: "ArrowRight" });
  expect(useHive.getState().splitPercent).toBe(80);
  fireEvent.keyDown(divider, { key: "ArrowLeft" });
  expect(useHive.getState().splitPercent).toBe(78);
});
