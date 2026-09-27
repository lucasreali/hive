import { ArrowUpIcon, PaperclipIcon, StopIcon, XIcon } from "@phosphor-icons/react";
import {
  type DragEvent,
  type KeyboardEvent,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  type ChatDraft,
  type ChatMode,
  type ChatModel,
  EMPTY_DRAFT,
  setDraft,
  setMentioning,
  useHive,
} from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { imageUrl } from "./ConversationView";
import { ICON } from "./icons";
import { FILE_LIMIT, fuzzy } from "./Palette";

/** The permission modes offered (7.3): never `bypassPermissions`. */
export const MODES: { value: ChatMode; label: string }[] = [
  { value: "default", label: "Default" },
  { value: "accept_edits", label: "Accept edits" },
  { value: "plan", label: "Plan" },
];

const NO_COMMANDS: string[] = [];
const NO_MODELS: ChatModel[] = [];
const NO_FILES: string[] = [];

/** A worktree listing's files and the folders holding them (ending in `/`), sorted. */
export function mentionPaths(files: string[]): string[] {
  const all = new Set<string>();
  for (const file of files) {
    for (let i = file.indexOf("/"); i >= 0; i = file.indexOf("/", i + 1)) {
      all.add(file.slice(0, i + 1));
    }
    all.add(file);
  }
  return [...all].sort();
}

/** The paths matching `query` fuzzily, the best first (ties keep their order), at most `FILE_LIMIT`. */
export function rankPaths(paths: string[], query: string): string[] {
  return paths
    .map((path) => ({ path, score: fuzzy(query, path) }))
    .filter((e): e is { path: string; score: number } => e.score !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, FILE_LIMIT)
    .map((e) => e.path);
}

/** A path as mentioned: quoted when it holds a space, as Claude's terminal writes it. */
export const mention = (path: string) => (/\s/.test(path) ? `@"${path}"` : `@${path}`);

/**
 * The service's limits (`hive::chat::MAX_IMAGES`, `MAX_IMAGE_DATA`): at most 10 images per
 * message, at most 3 MiB of base64 together, so the message fits in one frame (4 MiB). The
 * service checks them again, and the type by content.
 */
export const MAX_IMAGES = 10;
export const MAX_IMAGE_DATA = 3 << 20;
const IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const TOO_MANY = "At most 10 images, 3 MiB together, can be sent at once.";
const NOT_IMAGES = "Only PNG, JPEG, GIF and WebP images can be added.";

/** Numbers attached images, so two equal ones are told apart. */
let attached = 0;

/** A file's bytes as base64. */
export async function base64(file: Blob): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  // In slices: one call with every byte would overflow the stack.
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/**
 * A chat's input (7.3), one box like Zed's agent panel: thumbnails and errors on top, the
 * message (growing with its text up to a limit), then a toolbar with Attach image on the left
 * and the mode selector and Send on the right. Enter sends, Shift+Enter starts a new line;
 * images are picked, pasted or dropped in and shown as thumbnails until sent. While a turn runs, Send turns into Stop, and
 * Esc stops too; closed (or not started yet) it is disabled. The draft (text, images, caret) is
 * kept in the store by chat (8.14), so it comes back when the tab shows again. Typing `/`
 * lists the chat's slash commands that start with what follows it: ↑/↓ move, Enter or Tab
 * picks, Esc hides the list. Typing `@` lists the files and folders of the chat's worktree the
 * same way, fuzzy-matched, and picking one puts in its `@path` (8.12): Claude reads it itself.
 * Like Claude's terminal (8.8), ↑ on the first line brings back the
 * messages sent in this chat (↓ walks back to what was being typed), and Ctrl+C with nothing
 * selected stops a running turn or, idle, clears the composer. The mode selector shows the service's mode and asks it for another.
 * The model selector (8.9) lists the models Claude offers and shows the one the chat runs; a
 * pick asks the service, which follows once Claude takes it (a refusal is an error entry).
 * Until the service names the listed model that runs the chat, it shows the model, else "Model".
 */
export function ChatComposer({ chat }: { chat: number }) {
  const draft = useHive((s) => s.drafts[chat] ?? EMPTY_DRAFT);
  const { text, images } = draft;
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  // Where the caret is, for the `@` list.
  const [cursor, setCursor] = useState(draft.end);
  // The draft the list was hidden for (Esc); typing shows it again.
  const [hidden, setHidden] = useState<string | null>(null);
  // ↑/↓ history (8.8): how far back (1 = the last message sent) and what was being typed.
  const [recall, setRecall] = useState<{ back: number; typed: string } | null>(null);
  // Where to put the caret once the text brought back by ↑/↓ shows.
  const caret = useRef<number | null>(null);
  const id = useId();
  const field = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const busy = useHive((s) => !!s.chats[chat]?.status?.busy);
  const ready = useHive((s) => !!s.chats[chat]?.opened && !s.chats[chat]?.closed);
  const mode = useHive((s) => s.chats[chat]?.status?.mode ?? s.chats[chat]?.opened?.mode);
  const commands = useHive((s) => s.chats[chat]?.opened?.commands ?? NO_COMMANDS);
  const models = useHive((s) => s.chats[chat]?.opened?.models ?? NO_MODELS);
  const choice = useHive((s) => s.chats[chat]?.status?.choice ?? null);
  const model = useHive((s) => s.chats[chat]?.status?.model ?? s.chats[chat]?.opened?.model);
  const offered = models.map(({ value, name }) => ({ value, label: name }));
  const unknown = { value: "", label: model ?? "Model" };
  const cwd = useHive((s) => s.chats[chat]?.cwd);
  // Only a worktree's files are offered, relative to it: the chat must run at its root.
  const worktree = useHive((s) =>
    Object.values(s.projects ?? {}).some((p) => p.worktrees.some((w) => w.path === cwd))
      ? (cwd as string)
      : null,
  );
  const listed = useHive((s) =>
    worktree && s.worktreeFiles?.path === worktree ? s.worktreeFiles.files : NO_FILES,
  );
  const paths = useMemo(() => mentionPaths(listed), [listed]);
  const typed = /^\/\S*$/.test(text) && text !== hidden ? text.slice(1) : null;
  // The `@` word the caret ends, if any.
  const word =
    worktree && typed === null && text !== hidden
      ? (/(?:^|\s)@([^\s@"]*)$/.exec(text.slice(0, cursor))?.[1] ?? null)
      : null;
  const matches =
    typed !== null
      ? commands.filter((c) => c.startsWith(typed))
      : word !== null
        ? rankPaths(paths, word)
        : [];
  const at = Math.min(active, matches.length - 1);
  const empty = text.trim() === "" && images.length === 0;
  const edit = (value: string, caret = value.length) => {
    setDraft(chat, { text: value });
    setCursor(caret);
    setActive(0);
    setRecall(null);
  };
  /** Shows message `back` of the history (0 = what was being typed), caret at its end. */
  const show = (back: number, value: string, typed: string) => {
    setDraft(chat, { text: value });
    setRecall(back === 0 ? null : { back, typed });
    caret.current = value.length;
    setCursor(value.length);
  };
  const setImages = (next: ChatDraft["images"]) => setDraft(chat, { images: next });
  /** Puts in the picked command, or the picked path in place of the `@` word (a file then a space). */
  const pick = (picked: string) => {
    if (word === null) return edit(`/${picked} `);
    const space = picked.endsWith("/") ? "" : " ";
    const before = `${text.slice(0, cursor - word.length - 1)}${mention(picked)}${space}`;
    edit(`${before}${text.slice(cursor)}`, before.length);
    caret.current = before.length;
  };
  // The `@` list's worktree is watched while it shows (`followPanel`).
  const mentioning = word !== null ? worktree : null;
  useEffect(() => {
    if (!mentioning) return;
    setMentioning(mentioning, true);
    return () => setMentioning(mentioning, false);
  }, [mentioning]);
  // The active row stays in view as ↑/↓ move through a long list.
  useEffect(() => {
    document.getElementById(`${id}-${at}`)?.scrollIntoView?.({ block: "nearest" });
  }, [id, at]);
  const send = () => {
    if (!ready || busy || empty) return;
    const sent = images.map(({ media_type, data }) => ({ media_type, data }));
    transport.chatSend(chat, text, sent).catch((e) => setError(String(e)));
    setDraft(chat, null);
    setActive(0);
    setRecall(null);
    setError(null);
  };
  const add = async (files: File[]) => {
    if (files.some((file) => !IMAGE_TYPES.includes(file.type))) return setError(NOT_IMAGES);
    // A huge file is refused before it is read into memory.
    const bytes = files.reduce((sum, file) => sum + file.size, 0);
    if (bytes > MAX_IMAGE_DATA) return setError(TOO_MANY);
    const read = await Promise.all(
      files.map(async (file) => ({
        key: ++attached,
        media_type: file.type,
        data: await base64(file),
      })),
    );
    // Read now: the draft may have changed (or the tab switched) while the files were read.
    const next = [...(useHive.getState().drafts[chat]?.images ?? []), ...read];
    const size = next.reduce((sum, image) => sum + image.data.length, 0);
    if (next.length > MAX_IMAGES || size > MAX_IMAGE_DATA) return setError(TOO_MANY);
    setImages(next);
    setError(null);
  };
  const keys = (event: KeyboardEvent) => {
    if (event.nativeEvent.isComposing) return;
    const listed = matches.length > 0;
    const move = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (listed && move) {
      event.preventDefault();
      setActive((at + move + matches.length) % matches.length);
    } else if (listed && (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey))) {
      event.preventDefault();
      pick(matches[at]);
    } else if (move && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey) {
      history(event, move);
    } else if (
      event.ctrlKey &&
      !event.altKey &&
      !event.metaKey &&
      event.key.toLowerCase() === "c"
    ) {
      interrupt(event);
    } else if (event.key === "Escape" && listed) {
      setHidden(text);
    } else if (event.key === "Escape" && busy) {
      void transport.chatInterrupt(chat);
    } else if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  };
  /**
   * ↑ on the caret's first line brings back the message sent before the one shown, ↓ on its
   * last line the one after, and past the newest what was being typed (8.8). The history is
   * this chat's user messages, text only. Elsewhere the keys move the caret as usual.
   */
  const history = (event: KeyboardEvent, move: number) => {
    const el = event.currentTarget as HTMLTextAreaElement;
    const edge =
      move < 0
        ? !text.slice(0, el.selectionStart).includes("\n")
        : !text.slice(el.selectionEnd).includes("\n");
    if (!edge || (move > 0 && !recall)) return;
    const sent = (useHive.getState().chats[chat]?.entries ?? [])
      .filter((entry) => entry.kind === "user" && entry.text !== "")
      .map((entry) => entry.text);
    const back = (recall?.back ?? 0) - move;
    if (back > sent.length) return;
    event.preventDefault();
    const typed = recall?.typed ?? text;
    show(back, back === 0 ? typed : sent[sent.length - back], typed);
  };
  /** Ctrl+C: copies a selection; else stops a running turn, or clears the composer (8.8). */
  const interrupt = (event: KeyboardEvent) => {
    const el = event.currentTarget as HTMLTextAreaElement;
    if (el.selectionStart !== el.selectionEnd) return;
    event.preventDefault();
    if (busy) {
      void transport.chatInterrupt(chat);
      return;
    }
    setDraft(chat, null);
    setRecall(null);
  };
  // The message grows with its text; CSS caps it, then it scrolls.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the height follows the text, the caret ↑/↓.
  useLayoutEffect(() => {
    const el = field.current as HTMLTextAreaElement;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
    if (caret.current !== null) el.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  }, [text, recall]);
  // The caret is kept when the composer goes (its tab hides) and comes back when it shows again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only when it mounts and unmounts.
  useLayoutEffect(() => {
    const el = field.current as HTMLTextAreaElement;
    el.setSelectionRange(draft.start, draft.end);
    return () => setDraft(chat, { start: el.selectionStart, end: el.selectionEnd });
  }, []);
  const files = (event: DragEvent) => event.dataTransfer.types.includes("Files");
  return (
    <form
      className="chat-composer"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
      onDragOver={(event) => {
        if (ready && files(event)) event.preventDefault();
      }}
      onDrop={(event) => {
        if (!ready || !files(event)) return;
        event.preventDefault();
        void add([...event.dataTransfer.files]);
      }}
    >
      {matches.length > 0 && (
        <div
          className="select-list chat-commands"
          role="listbox"
          id={id}
          aria-label={word !== null ? "Files" : "Commands"}
          // The focus stays in the message.
          onMouseDown={(event) => event.preventDefault()}
        >
          {matches.map((command, i) => (
            // biome-ignore lint/a11y/useKeyWithClickEvents: the message field handles the keys.
            <div
              key={command}
              id={`${id}-${i}`}
              role="option"
              tabIndex={-1}
              aria-selected={i === at}
              data-active={i === at}
              onMouseMove={() => setActive(i)}
              onClick={() => pick(command)}
            >
              {word !== null ? `@${command}` : `/${command}`}
            </div>
          ))}
        </div>
      )}
      {images.length > 0 && (
        <ul className="chat-attachments" aria-label="Images to send">
          {images.map((image, i) => (
            <li key={image.key}>
              <img src={imageUrl(image)} alt={`Attachment ${i + 1}`} />
              <button
                type="button"
                className="ghost"
                title={`Remove image ${i + 1}`}
                onClick={() => setImages(images.filter((other) => other !== image))}
              >
                <XIcon {...ICON} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p className="files-error chat-composer-error" role="alert">
          {error}
        </p>
      )}
      <textarea
        ref={field}
        aria-label="Message"
        aria-controls={matches.length > 0 ? id : undefined}
        aria-activedescendant={matches.length > 0 ? `${id}-${at}` : undefined}
        placeholder={ready ? "Message Claude — / for commands" : undefined}
        rows={3}
        value={text}
        disabled={!ready}
        onChange={(event) => edit(event.target.value, event.target.selectionEnd)}
        onSelect={(event) => setCursor(event.currentTarget.selectionEnd)}
        onKeyDown={keys}
        onPaste={(event) => {
          const pasted = [...event.clipboardData.files];
          if (pasted.length === 0) return;
          event.preventDefault();
          void add(pasted);
        }}
      />
      <div className="chat-toolbar">
        <button
          type="button"
          className="ghost chat-icon"
          aria-label="Attach image"
          title="Attach image"
          disabled={!ready}
          onClick={() => picker.current?.click()}
        >
          <PaperclipIcon {...ICON} />
        </button>
        <input
          ref={picker}
          type="file"
          accept={IMAGE_TYPES.join(",")}
          multiple
          hidden
          onChange={(event) => {
            const picked = [...(event.target.files ?? [])];
            // The same file can be picked again.
            event.target.value = "";
            if (picked.length > 0) void add(picked);
          }}
        />
        <Select
          className="chat-mode"
          aria-label="Permission mode"
          value={mode ?? "default"}
          options={MODES}
          disabled={!ready}
          onChange={(value) => void transport.chatSetMode(chat, value as ChatMode)}
        />
        {models.length > 0 && (
          <Select
            className="chat-mode chat-model"
            aria-label="Model"
            value={choice ?? ""}
            options={choice === null ? [unknown, ...offered] : offered}
            disabled={!ready}
            // "" (the model before one is listed) is never picked: it shows only while current.
            onChange={(value) => void transport.chatSetModel(chat, value)}
          />
        )}
        {busy ? (
          <button
            type="button"
            className="secondary chat-icon"
            aria-label="Stop"
            title="Stop the turn (Esc)"
            onClick={() => void transport.chatInterrupt(chat)}
          >
            <StopIcon {...ICON} weight="fill" />
          </button>
        ) : (
          <button
            type="submit"
            className="primary chat-icon"
            aria-label="Send"
            title="Send (Enter)"
            disabled={!ready || empty}
          >
            <ArrowUpIcon {...ICON} weight="bold" />
          </button>
        )}
      </div>
    </form>
  );
}
