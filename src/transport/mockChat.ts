import type {
  ChatAnswer,
  ChatEntry,
  ChatImage,
  ChatMode,
  ChatRequest,
  ServiceMessage,
} from "../store";

/** Delay between two scripted chat messages in the browser mock. */
export const CHAT_STEP_MS = 150;
export const MOCK_CHAT_MODEL = "claude-mock";
export const MOCK_CHAT_COMMANDS = ["compact", "clear", "review"];

/** `request` of a turn whose text holds the key (the first key found wins). */
export const MOCK_CHAT_REQUESTS: Record<
  "permission" | "question" | "plan",
  Omit<ChatRequest, "id">
> = {
  permission: {
    kind: "permission",
    tool: "Bash",
    detail: "rm -rf target",
    reason: "Bash commands need approval in the default mode",
    questions: [],
    plan: null,
  },
  question: {
    kind: "question",
    tool: "AskUserQuestion",
    detail: "",
    reason: null,
    questions: [
      {
        question: "Which language should I greet you in?",
        header: "Language",
        multi: false,
        options: [
          { label: "English", description: "Greet in English" },
          { label: "Portuguese", description: "Greet in Portuguese" },
        ],
      },
      {
        question: "Which files should I touch?",
        header: "Files",
        multi: true,
        options: [
          { label: "README.md", description: "The readme" },
          { label: "CONTRIBUTING.md", description: "The contributing guide" },
        ],
      },
    ],
    plan: null,
  },
  plan: {
    kind: "plan",
    tool: "ExitPlanMode",
    detail: "",
    reason: null,
    questions: [],
    plan: "## Add CONTRIBUTING.md\n1. Create CONTRIBUTING.md.\n2. Add a section about commit messages.",
  },
};

/** What the mock answers for each kind of answer. */
export function describeAnswer(answer: ChatAnswer): string {
  switch (answer.kind) {
    case "allow":
      return "Allowed, so I went ahead.";
    case "deny":
      return `Denied${answer.message ? `: ${answer.message}` : ""}. I stopped.`;
    case "answers":
      return `You chose: ${answer.answers.map((a) => a.join(", ")).join(" / ")}.`;
    case "approve_plan":
      return `Plan approved${answer.accept_edits ? ", accepting edits" : ""}. Done.`;
    case "keep_planning":
      return `Planning again: ${answer.feedback}`;
  }
}

type MockChat = {
  cwd: string;
  mode: ChatMode;
  session: string;
  /** The last entry id given. */
  last: number;
  busy: boolean;
  timers: ReturnType<typeof setTimeout>[];
  pending: string | null;
  /** Opened with `resume`: the history is sent first. */
  resumed: boolean;
};

/**
 * A scripted chat (7.3b) standing in for the service's `hive::chat`, so the UI can be built and
 * tested without `claude`. Every turn: user → thinking → assistant → tool running → tool ok →
 * live assistant text → usage. Words in the turn's text add more: `permission`, `question` or
 * `plan` end the turn on a request (answered with `chat_request_gone` and a reply),
 * `subagent` adds an `Agent` call with its subagent's entries, `compact` a divider, `error`
 * a retry and an error entry, `crash` closes the chat with an error. The first chat in each
 * folder asks `confirm_chat_folder`.
 */
export function createMockChat(send: (message: ServiceMessage) => void, step = CHAT_STEP_MS) {
  const chats = new Map<number, MockChat>();
  const confirmed = new Set<string>();
  /** Chats waiting for `confirm_chat_folder`'s answer. */
  const waiting = new Map<number, MockChat>();
  let requests = 0;

  const status = (id: number, chat: MockChat, extra: { retry?: string; compacting?: boolean }) =>
    send({
      type: "chat_status",
      channel: id,
      chat: id,
      busy: chat.busy,
      mode: chat.mode,
      model: MOCK_CHAT_MODEL,
      retry: extra.retry ?? null,
      compacting: extra.compacting ?? false,
      api_key_source: null,
      session: chat.session,
    });
  const entry = (
    chat: MockChat,
    kind: ChatEntry["kind"],
    text: string,
    more: Partial<ChatEntry> = {},
  ): ChatEntry => ({
    id: ++chat.last,
    kind,
    text,
    tool: null,
    parent: null,
    status: null,
    output: null,
    image: null,
    ...more,
  });
  const entries = (id: number, list: ChatEntry[], replace_last = false) =>
    send({ type: "chat_entries", channel: id, chat: id, entries: list, replace_last });

  /** Runs `steps` one `step` apart, as long as the chat stays open. */
  const play = (id: number, chat: MockChat, steps: (() => void)[]) => {
    steps.forEach((run, i) => {
      chat.timers.push(setTimeout(() => chats.get(id) === chat && run(), step * (i + 1)));
    });
  };
  const finish = (id: number, chat: MockChat) => {
    entries(id, [entry(chat, "usage", "2.3 s · 40 output tokens · 12% context")]);
    chat.busy = false;
    status(id, chat, {});
  };
  const opened = (id: number, chat: MockChat) => {
    chats.set(id, chat);
    const { cwd, session, mode } = chat;
    send({
      type: "chat_opened",
      channel: id,
      chat: id,
      cwd,
      session,
      model: MOCK_CHAT_MODEL,
      mode,
      commands: MOCK_CHAT_COMMANDS,
      api_key_source: null,
    });
    status(id, chat, {});
    if (chat.resumed) {
      entries(id, [
        entry(chat, "user", "What are git worktrees?"),
        entry(chat, "assistant", "Several checkouts of one repository, each on its own branch."),
      ]);
    }
  };

  const turn = (id: number, chat: MockChat, text: string, images: ChatImage[]) => {
    const has = (word: string) => text.toLowerCase().includes(word);
    const steps: (() => void)[] = [];
    // Entries are made now, so their ids increase in the order they are sent.
    const show = (list: ChatEntry[], replace_last = false) =>
      steps.push(() => entries(id, list, replace_last));
    const tool = (name: string, summary: string, output: string, parent: string | null = null) => {
      const call = entry(chat, "tool", summary, { tool: name, parent, status: "running" });
      show([call]);
      show([{ ...call, status: "ok", output }]);
    };
    chat.busy = true;
    status(id, chat, {});
    const user = [entry(chat, "user", text, { image: images[0] ?? null })];
    user.push(...images.slice(1).map((image) => entry(chat, "user", "", { image })));
    entries(id, user);
    show([entry(chat, "thinking", "Let me look at the worktree first.")]);
    show([entry(chat, "assistant", "I'll list the files.")]);
    tool("Bash", "ls -la", "README.md\nsrc\n");
    if (has("subagent")) {
      const call = entry(chat, "tool", "Count the lines of notes.txt", {
        tool: "Agent",
        status: "running",
      });
      const parent = `toolu_mock_${call.id}`;
      show([call]);
      show([
        entry(chat, "user", "Count the lines of notes.txt.", { parent }),
        entry(chat, "assistant", "I'll read the file.", { parent }),
      ]);
      tool("Read", "notes.txt", "alpha\nbeta\n", parent);
      show([{ ...call, status: "ok", output: "notes.txt has 2 lines." }]);
    }
    if (has("compact")) {
      steps.push(() => status(id, chat, { compacting: true }));
      const divider = entry(chat, "divider", "Conversation compacted (150k tokens)");
      steps.push(() => {
        entries(id, [divider]);
        status(id, chat, {});
      });
    }
    // Live text: the reply grows in place.
    const reply = entry(chat, "assistant", "The worktree has");
    show([reply]);
    show([{ ...reply, text: "The worktree has a README.md and a src folder." }], true);
    if (has("crash")) {
      steps.push(() => {
        chats.delete(id);
        const error = "mock: claude exited with code 1";
        send({ type: "chat_closed", channel: id, chat: id, error });
      });
      return play(id, chat, steps);
    }
    if (has("error")) {
      steps.push(() => status(id, chat, { retry: "Retrying 2/10…" }));
      show([entry(chat, "error", "API Error: 529 overloaded")]);
    }
    const kind = (["permission", "question", "plan"] as const).find(has);
    if (!kind) {
      steps.push(() => finish(id, chat));
      return play(id, chat, steps);
    }
    const request = { id: `req_mock_${++requests}`, ...MOCK_CHAT_REQUESTS[kind] };
    steps.push(() => {
      chat.pending = request.id;
      send({ type: "chat_request", channel: id, chat: id, request });
    });
    play(id, chat, steps);
  };

  const later = (run: () => void) => setTimeout(run, 0);
  return {
    open(id: number, cwd: string, resume: string | null, mode: ChatMode | null) {
      const chat: MockChat = {
        cwd,
        mode: mode ?? "default",
        session: resume ?? `mock-chat-${id}`,
        last: 0,
        busy: false,
        timers: [],
        pending: null,
        resumed: resume !== null,
      };
      if (confirmed.has(cwd)) return void later(() => opened(id, chat));
      waiting.set(id, chat);
      later(() => send({ type: "confirm_chat_folder", channel: id, chat: id, cwd }));
    },
    confirm(id: number, cwd: string, accepted: boolean) {
      const chat = waiting.get(id);
      if (chat?.cwd !== cwd) return;
      waiting.delete(id);
      if (!accepted)
        return void later(() => send({ type: "chat_closed", channel: id, chat: id, error: null }));
      confirmed.add(cwd);
      later(() => opened(id, chat));
    },
    send(id: number, text: string, images: ChatImage[]) {
      const chat = chats.get(id);
      if (chat) later(() => turn(id, chat, text, images));
    },
    answer(id: number, request: string, answer: ChatAnswer) {
      const chat = chats.get(id);
      if (!chat) return;
      if (chat.pending !== request) {
        return void later(() => send({ type: "error", message: `no pending request ${request}` }));
      }
      chat.pending = null;
      if (answer.kind === "approve_plan" && answer.accept_edits) chat.mode = "accept_edits";
      later(() => {
        send({ type: "chat_request_gone", channel: id, chat: id, request });
        entries(id, [entry(chat, "assistant", describeAnswer(answer))]);
        finish(id, chat);
      });
    },
    interrupt(id: number) {
      const chat = chats.get(id);
      if (!chat?.busy) return;
      chat.timers.splice(0).forEach(clearTimeout);
      const pending = chat.pending;
      chat.pending = null;
      later(() => {
        if (pending) send({ type: "chat_request_gone", channel: id, chat: id, request: pending });
        entries(id, [entry(chat, "note", "Interrupted")]);
        chat.busy = false;
        status(id, chat, {});
      });
    },
    setMode(id: number, mode: ChatMode) {
      const chat = chats.get(id);
      if (!chat) return;
      chat.mode = mode;
      later(() => status(id, chat, {}));
    },
    close(id: number) {
      const chat = chats.get(id);
      chat?.timers.splice(0).forEach(clearTimeout);
      if (!chats.delete(id) && !waiting.delete(id)) return;
      later(() => send({ type: "chat_closed", channel: id, chat: id, error: null }));
    },
  };
}
