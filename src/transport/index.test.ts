import { expect, test } from "bun:test";
import { pickTransport, transport } from ".";
import { tauriTransport } from "./tauri";

test("the Tauri transport is used inside the app unless ?mock is set", async () => {
  expect(await pickTransport(true, "")).toBe(tauriTransport);
  expect(await pickTransport(true, "?mock")).not.toBe(tauriTransport);
  expect(await pickTransport(true, "?mock=mismatch")).not.toBe(tauriTransport);
  expect(await pickTransport(false, "")).not.toBe(tauriTransport);
});

test("a plain browser gets the mock transport", async () => {
  expect(transport).not.toBe(tauriTransport);
  expect(await pickTransport()).not.toBe(tauriTransport);
});
