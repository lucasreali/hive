import { PaperPlaneRightIcon, StopIcon, XIcon } from "@phosphor-icons/react";
import { type DragEvent, type KeyboardEvent, useState } from "react";
import { type ChatImage, useHive } from "../store";
import { transport } from "../transport";
import { imageUrl } from "./ConversationView";
import { ICON } from "./icons";

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

/** An image waiting to be sent; `key` tells two equal ones apart. */
type Attachment = ChatImage & { key: number };
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
 * A chat's input (7.3): Enter sends, Shift+Enter starts a new line; images are pasted or
 * dropped in and shown as thumbnails until sent. While a turn runs, Send turns into Stop; closed
 * (or not started yet) it is disabled. The draft is UI state.
 */
export function ChatComposer({ chat }: { chat: number }) {
  const [text, setText] = useState("");
  const [images, setImages] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const busy = useHive((s) => !!s.chats[chat]?.status?.busy);
  const ready = useHive((s) => !!s.chats[chat]?.opened && !s.chats[chat]?.closed);
  const empty = text.trim() === "" && images.length === 0;
  const send = () => {
    if (!ready || busy || empty) return;
    const sent = images.map(({ media_type, data }) => ({ media_type, data }));
    transport.chatSend(chat, text, sent).catch((e) => setError(String(e)));
    setText("");
    setImages([]);
    setError(null);
  };
  const add = async (files: File[]) => {
    if (files.some((file) => !IMAGE_TYPES.includes(file.type))) return setError(NOT_IMAGES);
    const read = await Promise.all(
      files.map(async (file) => ({
        key: ++attached,
        media_type: file.type,
        data: await base64(file),
      })),
    );
    const next = [...images, ...read];
    const size = next.reduce((sum, image) => sum + image.data.length, 0);
    if (next.length > MAX_IMAGES || size > MAX_IMAGE_DATA) return setError(TOO_MANY);
    setImages(next);
    setError(null);
  };
  const keys = (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    send();
  };
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
        aria-label="Message"
        placeholder={ready ? "Message Claude (Shift+Enter for a new line)" : undefined}
        rows={3}
        value={text}
        disabled={!ready}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={keys}
        onPaste={(event) => {
          const pasted = [...event.clipboardData.files];
          if (pasted.length === 0) return;
          event.preventDefault();
          void add(pasted);
        }}
      />
      {busy ? (
        <button
          type="button"
          className="secondary"
          title="Stop the turn"
          onClick={() => void transport.chatInterrupt(chat)}
        >
          <StopIcon {...ICON} /> Stop
        </button>
      ) : (
        <button type="submit" className="primary" disabled={!ready || empty}>
          <PaperPlaneRightIcon {...ICON} /> Send
        </button>
      )}
    </form>
  );
}
