// Recorded terminal output for the load test (1.11, #28): the mock transport replays it into
// every terminal with `?mock=load`. The default recording is generated here, deterministically,
// to look like Claude Code's TUI; `?mock=load&cast=<url>` replays a real capture instead.

/** One chunk of terminal output, `at` milliseconds after the recording starts. */
export type ReplayEvent = { at: number; data: string };

/**
 * Parses an asciinema cast (v2: absolute times, v3: intervals), the format of
 * `asciinema rec --command claude out.cast`. Only output ("o") events are kept.
 */
export function parseCast(text: string): ReplayEvent[] {
  const [head, ...lines] = text.split("\n").filter((l) => l.trim() && !l.startsWith("#"));
  const version = (JSON.parse(head ?? "null") as { version?: number } | null)?.version;
  if (version !== 2 && version !== 3) throw new Error(`not an asciinema v2/v3 cast: ${head}`);
  const events: ReplayEvent[] = [];
  let time = 0;
  for (const line of lines) {
    const [t, kind, data] = JSON.parse(line) as [number, string, string];
    time = version === 2 ? t : time + t;
    if (kind === "o") events.push({ at: Math.round(time * 1000), data });
  }
  return events;
}

/** The recording for `?mock=load`: the cast at `url` if given, the generated one otherwise. */
export async function loadReplay(url: string | null): Promise<ReplayEvent[]> {
  if (!url) return claudeRecording();
  const response = await fetch(url);
  if (!response.ok) throw new Error(`cannot fetch ${url}: ${response.status}`);
  return parseCast(await response.text());
}

/** The last line of the generated recording. */
export const RECORDING_END = "✻ Replay complete";

const WIDTH = 80;
const RESET = "\x1b[0m";
const DIM = "\x1b[2m";
const ORANGE = "\x1b[38;2;215;119;87m";
const GRAY = "\x1b[38;5;246m";
// Synchronized output (DEC 2026), which Claude Code wraps each redraw in.
const SYNC = "\x1b[?2026h";
const UNSYNC = "\x1b[?2026l";
const SPINNER = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
const WORDS = (
  "the login handler compares the hashed password before the session store is ready so the " +
  "first request after a restart fails I will move the check behind the store initialisation " +
  "and add a regression test that restarts the service between two logins"
).split(" ");

/** Ink's log-update: erase the previous dynamic region line by line, cursor back to column 1. */
const erase = (count: number) =>
  Array.from({ length: count }, (_, i) => `\x1b[2K${i < count - 1 ? "\x1b[1A" : ""}`).join("") +
  "\x1b[G";

/** The verb with Claude Code's moving shimmer: one truecolor per character. */
function shimmer(text: string, frame: number): string {
  return [...text]
    .map((c, i) => {
      const glow = Math.max(0, 60 - Math.abs(i - (frame % (text.length + 8))) * 20);
      return `\x1b[38;2;${215 + Math.min(40, glow)};${119 + glow};${87 + glow}m${c}`;
    })
    .join("");
}

function inputBox(): string[] {
  const bar = "─".repeat(WIDTH - 2);
  return [
    `${GRAY}╭${bar}╮${RESET}`,
    `${GRAY}│${RESET} > ${" ".repeat(WIDTH - 5)}${GRAY}│${RESET}`,
    `${GRAY}╰${bar}╯${RESET}`,
    `  ${DIM}? for shortcuts${RESET}${" ".repeat(WIDTH - 24)}${DIM}◯ IDE${RESET}`,
  ];
}

function todos(done: number): string[] {
  const items = ["Reproduce the failure", "Fix the order", "Add a test", "Run the suite"];
  return items.map((item, i) =>
    i < done
      ? `  ${DIM}☒ \x1b[9m${item}${RESET}`
      : `  ${i === done ? "\x1b[1m" : ""}☐ ${item}${RESET}`,
  );
}

/** A diff as Claude Code shows an Edit: line numbers, truecolor red/green backgrounds. */
function diff(lines: number, seed: number): string[] {
  return Array.from({ length: lines }, (_, i) => {
    const n = String(seed * 10 + i).padStart(4);
    const code = `    let session = store.get(&id).await?; // step ${i}`.padEnd(WIDTH - 12);
    if (i % 7 === 3) return `  ${n} \x1b[48;2;92;36;40m\x1b[38;2;230;200;200m-${code}${RESET}`;
    if (i % 7 === 4) return `  ${n} \x1b[48;2;33;76;46m\x1b[38;2;200;230;200m+${code}${RESET}`;
    return `  ${n} ${DIM} ${code}${RESET}`;
  });
}

/**
 * About 20 s of a Claude Code session, three turns of: prompt, spinner (redrawn at 12.5 Hz with
 * a todo list and the input box), streamed answer (the whole dynamic region redrawn per
 * chunk), a tool call with a long diff flushed in PTY-sized chunks. Deterministic.
 */
export function claudeRecording(): ReplayEvent[] {
  const events: ReplayEvent[] = [];
  let at = 0;
  let region = 0; // lines of the dynamic region on screen, trailing newline included
  let frame = 0;
  let tokens = 0;
  const emit = (data: string) => events.push({ at, data });
  /** Writes `above` as finished output, then redraws the dynamic region, as Ink does. */
  const draw = (dynamic: string[], above: string[] = []) => {
    const lines = [...above, ...dynamic];
    emit(`${SYNC}${erase(region)}${lines.map((l) => `${l}\r\n`).join("")}${UNSYNC}`);
    region = dynamic.length + 1;
  };
  const status = (verb: string, done: number) => [
    `${ORANGE}${SPINNER[frame % SPINNER.length]}${RESET} ${shimmer(`${verb}…`, frame)} ` +
      `${GRAY}(${Math.floor(at / 1000)}s · ↑ ${(tokens / 1000).toFixed(1)}k tokens · esc to interrupt)${RESET}`,
    ...todos(done),
    "",
    ...inputBox(),
  ];
  const spin = (verb: string, ms: number, done: number) => {
    for (const end = at + ms; at < end; at += 80, frame++, tokens += 37) draw(status(verb, done));
  };

  draw(inputBox(), [
    `${ORANGE}╭${"─".repeat(48)}╮${RESET}`,
    `${ORANGE}│${RESET} ✻ Welcome to Claude Code!${" ".repeat(22)}${ORANGE}│${RESET}`,
    `${ORANGE}╰${"─".repeat(48)}╯${RESET}`,
  ]);
  for (let turn = 0; turn < 3; turn++) {
    at += 400;
    draw(status("Thinking", turn), [
      "",
      `\x1b[48;5;237m> fix the failing login test (${turn + 1}/3)${RESET}`,
    ]);
    spin("Thinking", 2000, turn);
    // The answer streams in word bursts; every chunk redraws the growing message.
    const answer: string[] = [];
    let line = "⏺ ";
    for (let w = 0; w < 90; w++) {
      const word = WORDS[(w + turn * 11) % WORDS.length] as string;
      if (line.length + word.length > WIDTH - 2) {
        answer.push(line);
        line = "  ";
      }
      line += `${word} `;
      if (w % 2 === 1) {
        at += 33;
        frame++;
        draw([...answer.slice(-20), line, "", ...status("Writing", turn)]);
      }
    }
    draw(status("Running", turn), [...answer, line]);
    spin("Running", 1000, turn);
    // The tool result, written in the 4 KiB reads a PTY delivers.
    const block = [
      `${ORANGE}⏺${RESET} \x1b[1mUpdate${RESET}(src/login.rs)`,
      `  ⎿  Updated src/login.rs with 12 additions and 9 removals`,
      ...diff(80, turn),
    ]
      .map((l) => `${l}\r\n`)
      .join("");
    emit(`${SYNC}${erase(region)}`);
    region = 0;
    for (let i = 0; i < block.length; i += 4096) {
      at += 2;
      emit(block.slice(i, i + 4096));
    }
    draw(status("Running", turn + 1));
    spin("Running", 1500, turn + 1);
  }
  emit(`${erase(region)}${RECORDING_END}\r\n`);
  return events;
}
