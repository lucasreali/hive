import { ArrowUpIcon, PaperclipIcon, StopIcon, XIcon } from "@phosphor-icons/react";
import {
  type DragEvent,
  type KeyboardEvent,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { type ChatDraft, type ChatMode, EMPTY_DRAFT, setDraft, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { imageUrl } from "./ConversationView";
import { ICON } from "./icons";

/** The permission modes offered (7.3): never `bypassPermissions`. */
export const MODES: { value: ChatMode; label: string }[] = [
  { value: "default", label: "Default" },
  { value: "accept_edits", label: "Accept edits" },
  { value: "plan", label: "Plan" },
];

const NO_COMMANDS: string[] = [];

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
 * picks, Esc hides the list. The mode selector shows the service's mode and asks it for another.
 */
export function ChatComposer({ chat }: { chat: number }) {
  const draft = useHive((s) => s.drafts[chat] ?? EMPTY_DRAFT);
  const { text, images } = draft;
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  // The draft the list was hidden for (Esc); typing shows it again.
  const [hidden, setHidden] = useState<string | null>(null);
  const id = useId();
  const field = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const busy = useHive((s) => !!s.chats[chat]?.status?.busy);
  const ready = useHive((s) => !!s.chats[chat]?.opened && !s.chats[chat]?.closed);
  const mode = useHive((s) => s.chats[chat]?.status?.mode ?? s.chats[chat]?.opened?.mode);
  const commands = useHive((s) => s.chats[chat]?.opened?.commands ?? NO_COMMANDS);
  const typed = /^\/\S*$/.test(text) && text !== hidden ? text.slice(1) : null;
  const matches = typed === null ? [] : commands.filter((c) => c.startsWith(typed));
  const at = Math.min(active, matches.length - 1);
  const empty = text.trim() === "" && images.length === 0;
  const edit = (value: string) => {
    setDraft(chat, { text: value });
    setActive(0);
  };
  const setImages = (next: ChatDraft["images"]) => setDraft(chat, { images: next });
  const pick = (command: string) => edit(`/${command} `);
  const send = () => {
    if (!ready || busy || empty) return;
    const sent = images.map(({ media_type, data }) => ({ media_type, data }));
    transport.chatSend(chat, text, sent).catch((e) => setError(String(e)));
    setDraft(chat, null);
    setActive(0);
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
    } else if (event.key === "Escape" && listed) {
      setHidden(text);
    } else if (event.key === "Escape" && busy) {
      void transport.chatInterrupt(chat);
    } else if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  };
  // The message grows with its text; CSS caps it, then it scrolls.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the height follows the text.
  useLayoutEffect(() => {
    const el = field.current as HTMLTextAreaElement;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [text]);
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
          aria-label="Commands"
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
              /{command}
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
        onChange={(event) => edit(event.target.value)}
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
