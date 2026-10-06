import { expect } from "bun:test";
import { fireEvent, screen } from "@testing-library/react";

/**
 * The search field named `name` has the app's clear button (14.5): absent while the field is
 * empty, there with text; a click empties the field and leaves the focus in it.
 */
export function expectClearButton(name: string) {
  const field = screen.getByRole("searchbox", { name }) as HTMLInputElement;
  const clear = () => screen.queryByRole("button", { name: "Clear search" });
  expect(clear()).toBeNull();
  fireEvent.change(field, { target: { value: "zzz" } });
  const button = clear();
  expect(button).not.toBeNull();
  if (button) {
    // Pressing it does not take the focus from the field.
    expect(fireEvent.mouseDown(button)).toBe(false);
    fireEvent.click(button);
  }
  expect(field.value).toBe("");
  expect(document.activeElement).toBe(field);
  expect(clear()).toBeNull();
}
