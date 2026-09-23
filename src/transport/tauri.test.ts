import { afterEach, expect, test } from "bun:test";
import type { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import type { ServiceMessage } from "../store";
import { tauriTransport } from "./tauri";

type Args = Record<string, unknown>;

function record(result: unknown = null) {
  const calls: [string, Args][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args as Args]);
    return result;
  });
  return calls;
}

afterEach(clearMocks);

test("connect hands a channel to Rust and delivers its messages", async () => {
  const calls = record();
  const received: ServiceMessage[] = [];
  const welcome: ServiceMessage = { type: "welcome", version: "0.1.0", distro: "Ubuntu" };
  await tauriTransport.connect((m) => received.push(m));
  const [[cmd, args]] = calls;
  expect(cmd).toBe("connect");
  (args.onMessage as Channel<ServiceMessage>).onmessage(welcome);
  expect(received).toEqual([welcome]);
});

test("each terminal gets its own byte channel", async () => {
  const calls = record(7);
  const received: Uint8Array[] = [];
  expect(await tauriTransport.openTerminal("/w", 80, 24, (b) => received.push(b))).toBe(7);
  const [[cmd, { onData, ...args }]] = calls;
  expect([cmd, args]).toEqual(["open_terminal", { cwd: "/w", cols: 80, rows: 24 }]);
  (onData as Channel<ArrayBuffer>).onmessage(new Uint8Array([104, 105]).buffer);
  expect(received).toEqual([new Uint8Array([104, 105])]);
});

test("terminal actions call their commands", async () => {
  const calls = record();
  await tauriTransport.writeTerminal(7, "ls\r");
  await tauriTransport.resizeTerminal(7, 100, 30);
  await tauriTransport.closeTerminal(7);
  expect(calls).toEqual([
    ["write_terminal", { id: 7, data: "ls\r" }],
    ["resize_terminal", { id: 7, cols: 100, rows: 30 }],
    ["close_terminal", { id: 7 }],
  ]);
});
