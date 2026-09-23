import type { ServiceMessage } from "../store";
import type { Transport } from ".";

const PROMPT = "mock$ ";

/**
 * A fake service for the browser (`bun run dev`, Playwright): it welcomes the UI, and each
 * terminal shows a prompt, echoes what is typed, repeats the line on Enter and exits on `exit`.
 * Service messages arrive asynchronously, as they do from the real service.
 */
export function createMockTransport(): Transport {
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
      later({ type: "welcome", version: "mock" });
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
