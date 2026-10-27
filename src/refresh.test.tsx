import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";
import { useRefresh } from "./refresh";
import { initialState, setFocused, setVisible, useHive } from "./store";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

function Refreshing(props: { ask: () => void; ms: number | null; now: boolean; dep: string }) {
  useRefresh(props.ask, props.ms, props.now, [props.dep]);
  return null;
}

test("a refresh pauses while the window is hidden and asks once on its return", () => {
  const every = spyOn(globalThis, "setInterval");
  const stop = spyOn(globalThis, "clearInterval");
  const ask = mock(() => {});
  const timers = () => every.mock.calls.filter(([, ms]) => ms === 1000);
  try {
    const shown = render(<Refreshing ask={ask} ms={1000} now dep="a" />);
    expect(ask).toHaveBeenCalledTimes(1);
    const [tick] = timers()[0] as [() => void, number];
    tick();
    expect(ask).toHaveBeenCalledTimes(2);
    // Merely unfocused it goes on.
    act(() => setFocused(false));
    expect(timers()).toHaveLength(1);
    act(() => setVisible(false));
    expect(stop).toHaveBeenCalledTimes(1);
    expect(ask).toHaveBeenCalledTimes(2);
    act(() => setVisible(true));
    expect(ask).toHaveBeenCalledTimes(3);
    expect(timers()).toHaveLength(2);
    // New deps start again, asking at once.
    shown.rerender(<Refreshing ask={ask} ms={1000} now dep="b" />);
    expect(ask).toHaveBeenCalledTimes(4);
  } finally {
    every.mockRestore();
    stop.mockRestore();
  }
});

test("without `now` it asks only on return; with no interval, never", () => {
  const every = spyOn(globalThis, "setInterval");
  const ask = mock(() => {});
  try {
    const shown = render(<Refreshing ask={ask} ms={1000} now={false} dep="a" />);
    expect(ask).not.toHaveBeenCalled();
    act(() => setVisible(false));
    act(() => setVisible(true));
    expect(ask).toHaveBeenCalledTimes(1);
    // Started again while visible: no ask until a tick.
    shown.rerender(<Refreshing ask={ask} ms={1000} now={false} dep="b" />);
    expect(ask).toHaveBeenCalledTimes(1);
    const calls = every.mock.calls.length;
    shown.rerender(<Refreshing ask={ask} ms={null} now dep="b" />);
    act(() => setVisible(false));
    act(() => setVisible(true));
    expect(ask).toHaveBeenCalledTimes(1);
    expect(every.mock.calls).toHaveLength(calls);
  } finally {
    every.mockRestore();
  }
});
