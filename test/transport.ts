import { afterAll } from "bun:test";
import { initialState, useHive } from "../src/store";
import { transport } from "../src/transport";

/**
 * Leaves the store as the calling file found it. The mock transport is one for every file:
 * once the file's tests are done it stops answering into the store, so its last answers (a
 * terminal's exit, a handshake) land nowhere instead of in a later file's tests.
 */
export function detachAfterAll() {
  afterAll(async () => {
    await transport.connect(() => {});
    await new Promise((resolve) => setTimeout(resolve, 0));
    useHive.setState(initialState, true);
  });
}
