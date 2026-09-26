import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NumberInput } from "./NumberInput";

afterEach(cleanup);

function field(value: string, step?: number) {
  const onChange = mock((_: string) => {});
  render(
    <NumberInput value={value} onChange={onChange} min={2} max={60} step={step} label="Delay" />,
  );
  return {
    onChange,
    input: screen.getByRole("spinbutton") as HTMLInputElement,
    up: screen.getByRole("button", { name: "Increase Delay" }) as HTMLButtonElement,
    down: screen.getByRole("button", { name: "Decrease Delay" }) as HTMLButtonElement,
  };
}

test("the buttons step by one and typing passes the text through", () => {
  const { onChange, input, up, down } = field("10");
  expect(input.value).toBe("10");
  expect([input.min, input.max, input.step]).toEqual(["2", "60", "1"]);
  fireEvent.click(up);
  expect(onChange).toHaveBeenLastCalledWith("11");
  fireEvent.click(down);
  expect(onChange).toHaveBeenLastCalledWith("9");
  fireEvent.change(input, { target: { value: "42" } });
  expect(onChange).toHaveBeenLastCalledWith("42");
  expect(up.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
});

test("arrow keys step, other keys are left alone", () => {
  const { onChange, input } = field("10", 5);
  expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(false);
  expect(onChange).toHaveBeenLastCalledWith("15");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(onChange).toHaveBeenLastCalledWith("5");
  expect(fireEvent.keyDown(input, { key: "1" })).toBe(true);
  expect(onChange).toHaveBeenCalledTimes(2);
});

test("steps are clamped to the bounds, and a button is disabled at its bound", () => {
  let f = field("58", 5);
  fireEvent.click(f.up);
  expect(f.onChange).toHaveBeenLastCalledWith("60");
  cleanup();
  f = field("60");
  expect(f.up.disabled).toBe(true);
  expect(f.down.disabled).toBe(false);
  cleanup();
  f = field("2");
  expect(f.down.disabled).toBe(true);
  expect(f.up.disabled).toBe(false);
  cleanup();
  f = field("99");
  fireEvent.click(f.down);
  expect(f.onChange).toHaveBeenLastCalledWith("60");
});

test("a blank or non-numeric text steps from the nearer bound", () => {
  const { onChange, up, down } = field(" ");
  expect(up.disabled).toBe(false);
  expect(down.disabled).toBe(false);
  fireEvent.click(up);
  expect(onChange).toHaveBeenLastCalledWith("2");
  fireEvent.click(down);
  expect(onChange).toHaveBeenLastCalledWith("60");
});
