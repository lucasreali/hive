import { afterEach, expect, jest, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { initialState, showNotice, useHive } from "../store";
import { INFO_MS, Toasts } from "./Toasts";

afterEach(() => {
  cleanup();
  jest.useRealTimers();
  useHive.setState(initialState, true);
});

const texts = () =>
  screen.queryAllByText(/./, { selector: ".toast-text" }).map((t) => t.textContent);

test("the region is a polite live region, there before anything shows", () => {
  render(<Toasts />);
  const region = screen.getByRole("status", { name: "Messages" });
  expect([region.getAttribute("aria-live"), region.childElementCount]).toEqual(["polite", 0]);
});

test("a confirmation fades after INFO_MS; an error stays until its × is clicked", () => {
  jest.useFakeTimers();
  render(<Toasts />);
  act(() => {
    showNotice("error", "Cannot copy the path: denied");
    showNotice("info", "Copied /w");
  });
  expect(texts()).toEqual(["Cannot copy the path: denied", "Copied /w"]);
  const kinds = [...screen.getByRole("status", { name: "Messages" }).children].map((t) =>
    t.getAttribute("data-kind"),
  );
  expect(kinds).toEqual(["error", "info"]);
  act(() => jest.advanceTimersByTime(INFO_MS - 1));
  expect(texts()).toEqual(["Cannot copy the path: denied", "Copied /w"]);
  act(() => jest.advanceTimersByTime(1));
  expect(texts()).toEqual(["Cannot copy the path: denied"]);
  act(() => jest.advanceTimersByTime(60_000));
  expect(texts()).toEqual(["Cannot copy the path: denied"]);
  const toast = screen.getByText("Cannot copy the path: denied").parentElement as HTMLElement;
  fireEvent.click(within(toast).getByRole("button", { name: "Dismiss" }));
  expect(texts()).toEqual([]);
});

test("a confirmation dismissed early leaves no timer behind; the newest 3 show", () => {
  jest.useFakeTimers();
  render(<Toasts />);
  act(() => showNotice("info", "Copied a"));
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(jest.getTimerCount()).toBe(0);
  act(() => {
    for (const t of ["1", "2", "3", "4"]) showNotice("error", t);
  });
  expect(texts()).toEqual(["2", "3", "4"]);
});
