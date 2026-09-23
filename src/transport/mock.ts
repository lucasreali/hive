import type { ServiceMessage } from "../store";
import type { Transport } from ".";

const PROMPT = "mock$ ";

/** What the fake service answers on `connect`, picked with `?mock=<scenario>`. */
const HANDSHAKE: Record<string, ServiceMessage> = {
  mismatch: {
    type: "version_mismatch",
    protocol: 1,
    version: "0.0.0-mock",
    app_protocol: 1,
    app_version: "mock",
  },
  disconnected: { type: "disconnected", reason: "mock: the hive bridge exited" },
};
const WELCOME: ServiceMessage = { type: "welcome", version: "mock", distro: "Ubuntu" };

/**
 * A fake service for the browser (`bun run dev`, Playwright): it welcomes the UI, and each
 * terminal shows a prompt, echoes what is typed, repeats the line on Enter and exits on `exit`.
 * Service messages arrive asynchronously, as they do from the real service.
 * `scenario` ("mismatch" or "disconnected") answers `connect` with that failure instead.
 */
export function createMockTransport(scenario: string | null = null): Transport {
  let send: (message: ServiceMessage) => void = () => {};
  let last = 0;
  const terminals = new Map<number, { onData: (bytes: Uint8Array) => void; line: string }>();
  const encoder = new TextEncoder();
  const later = (message: ServiceMessage) => setTimeout(() => send(message), 0);
  const print = (id: number, text: string) => terminals.get(id)?.onData(encoder.encode(text));
  const exit = (id: number, code: number | null) => {
    if (terminals.delete(id)) later({ type: "terminal_exited", channel: id, code });
  };

  return {
    async connect(onMessage) {
      send = onMessage;
      later(HANDSHAKE[scenario ?? ""] ?? WELCOME);
    },
    async openTerminal(_cwd, _cols, _rows, onData) {
      const id = ++last;
      terminals.set(id, { onData, line: "" });
      later({ type: "terminal_opened", channel: id });
      setTimeout(() => print(id, PROMPT), 0);
      return id;
    },
    async writeTerminal(id, data) {
      const terminal = terminals.get(id);
      if (!terminal) return;
      for (const char of data) {
        if (char !== "\r") {
          terminal.line += char;
          print(id, char);
          continue;
        }
        const line = terminal.line;
        terminal.line = "";
        if (line === "exit") return exit(id, 0);
        print(id, `\r\n${line ? `${line}\r\n` : ""}${PROMPT}`);
      }
    },
    async resizeTerminal() {},
    async closeTerminal(id) {
      exit(id, null);
    },
  };
}
