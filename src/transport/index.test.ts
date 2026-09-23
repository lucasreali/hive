import { expect, test } from "bun:test";
import { pickTransport, transport } from ".";
import { tauriTransport } from "./tauri";

test("the Tauri transport is used inside the app unless ?mock is set", () => {
  expect(pickTransport(true, "")).toBe(tauriTransport);
  expect(pickTransport(true, "?mock")).not.toBe(tauriTransport);
  expect(pickTransport(true, "?mock=mismatch")).not.toBe(tauriTransport);
  expect(pickTransport(false, "")).not.toBe(tauriTransport);
});

test("a plain browser gets the mock transport", () => {
  expect(transport).not.toBe(tauriTransport);
  expect(pickTransport()).not.toBe(tauriTransport);
});
