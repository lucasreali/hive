import type { IDecoration, IMarker, Terminal } from "@xterm/xterm";

// Prompt marks (17.25): where a shell's prompts are in a terminal's scrollback, from its
// OSC 133 marks (fish 4 emits them; iTerm2's and Ghostty's shell integrations too). Pure view:
// a line's place exists only in xterm's buffer, so xterm keeps them, for ⌘↑ / ⌘↓ and a bar
// showing how each command ended. What the service acts on (long commands, notifications,
// progress) it reads itself (`crates/hive/src/osc.rs`).

/** An OSC 133 payload longer than this is ignored: terminal output is untrusted. */
export const MAX_MARK = 256;
/** A terminal keeps at most this many prompts, dropping the oldest. */
export const MAX_PROMPTS = 1000;

/** What an OSC 133 payload (after `133;`) marks: a prompt, a command's start or its end. */
export type Mark =
  | { kind: "prompt" }
  | { kind: "start" }
  | { kind: "end"; exit: number | null }
  | null;

/** `A` starts a prompt, `C` a command, `D[;<exit>]` ends it; anything else marks nothing. */
export function parseMark(data: string): Mark {
  if (data.length > MAX_MARK) return null;
  const [kind, exit] = data.split(";");
  if (kind === "A") return { kind: "prompt" };
  if (kind === "C") return { kind: "start" };
  if (kind !== "D") return null;
  return {
    kind: "end",
    exit: exit !== undefined && /^-?\d{1,9}$/.test(exit) ? Number(exit) : null,
  };
}

/** How a command ended, as its prompt's bar shows it; null: the shell gave no status. */
export type Status = "ok" | "failed" | null;

/** Draws a prompt's bar (`.shell-mark`), coloured by its command's `status`. */
export function renderMark(el: HTMLElement, status: Status): void {
  el.classList.add("shell-mark");
  if (status) el.dataset.status = status;
}

type Prompt = { marker: IMarker; decoration?: IDecoration; status?: Status };

/** A terminal's prompts, in order, and the jumps between them. */
export class Marks {
  private prompts: Prompt[] = [];
  /** The prompt whose command started and has not ended. */
  private running: Prompt | null = null;

  constructor(private readonly term: Terminal) {
    term.parser.registerOscHandler(133, (data) => {
      this.mark(parseMark(data));
      return true;
    });
  }

  /** Each prompt's line and its command's status (undefined until it ended). */
  list(): { line: number; status?: Status }[] {
    return this.prompts.map((p) => ({ line: p.marker.line, status: p.status }));
  }

  private mark(mark: Mark): void {
    if (mark?.kind === "prompt") {
      const prompt: Prompt = { marker: this.term.registerMarker(0) };
      this.running = null;
      this.prompts.push(prompt);
      // Gone from the scrollback, or erased while its command runs (`clear`'s `\x1b[2J`, 21.1):
      // gone from the list, and its command's end marks nothing.
      prompt.marker.onDispose(() => {
        this.prompts = this.prompts.filter((p) => p !== prompt);
        if (this.running === prompt) this.running = null;
      });
      if (this.prompts.length > MAX_PROMPTS) this.prompts[0]?.marker.dispose();
    } else if (mark?.kind === "start") {
      this.running = this.prompts.at(-1) ?? null;
    } else if (mark?.kind === "end" && this.running) {
      const prompt = this.running;
      this.running = null;
      const status: Status = mark.exit === null ? null : mark.exit === 0 ? "ok" : "failed";
      prompt.status = status;
      const decoration = this.term.registerDecoration({ marker: prompt.marker, width: 1 });
      decoration?.onRender((el) => renderMark(el, status));
      prompt.decoration = decoration;
    }
  }

  /**
   * ⌘K and the menu's Clear (21.1): the scrollback and the screen go, and the prompt the cursor
   * is at moves to the top whole, from its mark (fish's prompt of two lines keeps both), as
   * Terminal.app and Ghostty keep it. Nothing reaches the shell. Without a mark above the cursor
   * on the screen (no shell integration, a command running), xterm's own clear keeps the
   * cursor's line.
   */
  clear(): void {
    const buffer = this.term.buffer.active;
    const line = this.running ? undefined : this.prompts.at(-1)?.marker.line;
    const row = line === undefined ? -1 : line - buffer.baseY;
    if (buffer.type !== "normal" || row < 0 || row > buffer.cursorY) this.term.clear();
    // Scrolled up by its row (the lines above it are dropped, not kept), the cursor with it.
    else this.term.write(`${row ? `\x1b[${row}S\x1b[${row}A` : ""}\x1b[3J`);
  }

  /**
   * Brings the previous (-1) or next (1) prompt to the top of the view: the nearest above or
   * below its top line. A next prompt the view cannot bring that far up (it is on the last
   * screen), or none, scrolls to the end. False when there is nowhere to go.
   */
  jump(direction: -1 | 1): boolean {
    const buffer = this.term.buffer.active;
    const from = buffer.viewportY;
    const lines = this.prompts.map((p) => p.marker.line);
    const target =
      direction < 0 ? lines.filter((l) => l < from).pop() : lines.find((l) => l > from);
    if (target === undefined && direction < 0) return false;
    if (target === undefined || target >= buffer.baseY) {
      if (from === buffer.baseY) return false;
      this.term.scrollToBottom();
      return true;
    }
    this.term.scrollToLine(target);
    return true;
  }
}
