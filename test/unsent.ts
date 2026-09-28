import { spyOn } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { type Transport, transport } from "../src/transport";
import { tauriTransport } from "../src/transport/tauri";

/** What every command fails with under `unsent`: the app's error when the link is down. */
export const LINK_DOWN = "not connected to the hive service";

/**
 * Sends `method` through the Tauri transport with every command failing, as when the link is
 * down (9.21), until the returned function restores the mock service.
 */
export function unsent(method: keyof Transport): () => void {
  mockIPC(() => {
    throw LINK_DOWN;
  });
  const spy = spyOn(transport, method).mockImplementation(tauriTransport[method] as never);
  return () => {
    spy.mockRestore();
    clearMocks();
  };
}
