import {
  ArrowDownIcon,
  CheckIcon,
  CircleNotchIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, type ReactNode, useEffect, useRef, useState } from "react";
import type { ChatEntry, ChatImage, ToolStatus } from "../store";
import { ICON } from "./icons";
import { Markdown } from "./Markdown";

/** How a message's author is named: a chat's own, and a subagent's (6.10, nested ones). */
export type Labels = { user: string; assistant: string };
export const CHAT_LABELS: Labels = { user: "You", assistant: "Claude" };
export const SUBAGENT_LABELS: Labels = { user: "Prompt", assistant: "Subagent" };

const STATUS_ICON: Record<ToolStatus, ReactNode> = {
  running: <CircleNotchIcon className="tool-status spin" size={12} aria-label="running" />,
  ok: <CheckIcon className="tool-status" size={12} aria-label="done" />,
  error: <WarningCircleIcon className="tool-status" size={12} aria-label="failed" />,
};

/**
 * The entries in the order shown: a subagent's (same `parent`) together as one group, where its
 * first entry arrived (right after the `Agent` call that started it), so parallel subagents do
 * not interleave.
 */
export function ordered(entries: ChatEntry[]): ChatEntry[] {
  const groups = new Map<string, ChatEntry[]>();
  const rows: (ChatEntry | ChatEntry[])[] = [];
  for (const entry of entries) {
    if (entry.parent === null) {
      rows.push(entry);
      continue;
    }
    const group = groups.get(entry.parent) ?? [];
    if (group.length === 0) {
      groups.set(entry.parent, group);
      rows.push(group);
    }
    group.push(entry);
  }
  return rows.flat();
}

/** An image as the `src` of an `<img>`: a `data:` URL (the service checked its type). */
export const imageUrl = (image: ChatImage) => `data:${image.media_type};base64,${image.data}`;

/** An entry's image, small until clicked. */
function Picture({ image }: { image: ChatImage }) {
  const [large, setLarge] = useState(false);
  return (
    <button
      type="button"
      className="chat-image"
      title={large ? "Shrink the image" : "Enlarge the image"}
      aria-pressed={large}
      onClick={() => setLarge(!large)}
    >
      <img src={imageUrl(image)} alt="" />
    </button>
  );
}

/**
 * One entry, by its kind: Claude's messages as Markdown (8.6), everything else plain text (the
 * user's own messages too). Memoized: live text (7.3h)
 * replaces one entry, and the store keeps the others, so only its row renders again.
 */
const Row = memo(function Row({ entry, labels }: { entry: ChatEntry; labels: Labels }) {
  const names = entry.parent === null ? labels : SUBAGENT_LABELS;
  switch (entry.kind) {
    case "user":
    case "assistant":
      return (
        <>
          <span className="transcript-role">{names[entry.kind]}</span>
          <span className="transcript-text">
            {entry.kind === "assistant" ? <Markdown text={entry.text} /> : entry.text}
            {entry.image && <Picture image={entry.image} />}
          </span>
        </>
      );
    case "thinking":
      // Collapsed (7.3 decision): the summary opens it.
      return (
        <details className="transcript-text">
          <summary>Thinking</summary>
          {entry.text}
        </details>
      );
    case "tool": {
      const head = (
        <>
          <span className="transcript-role">
            {entry.status && STATUS_ICON[entry.status]}
            {entry.tool}
          </span>
          <span className="transcript-text">{entry.text}</span>
        </>
      );
      if (entry.output === null) return head;
      return (
        <details className="tool-details">
          <summary>{head}</summary>
          <pre className="tool-output">{entry.output}</pre>
          {entry.image && <Picture image={entry.image} />}
        </details>
      );
    }
    case "error":
      return (
        <>
          <span className="transcript-role">Error</span>
          <span className="transcript-text">{entry.text}</span>
        </>
      );
    default:
      // A note ("Interrupted"), a divider ("Conversation compacted") or a turn's usage.
      return <span className="transcript-text">{entry.text}</span>;
  }
});

/** How close to the bottom (px, about one line) a view still counts as at the bottom (8.13). */
const AT_BOTTOM = 24;

/**
 * A conversation's entries (6.10's subagent view and the chat, 7.3), newest last, virtualized;
 * the list keeps to the bottom while new ones arrive. `children` show above the entries (hints).
 * Scrolled up (8.13), it stays put and shows a "back to bottom" button and a bar with the user's
 * prompt the view's top belongs to. `initialOffset` starts the view at that offset instead of the
 * bottom; `onScroll` reports each scroll's offset and whether it is at the bottom (for 8.14).
 */
export function ConversationView({
  entries,
  labels,
  children,
  initialOffset,
  onScroll,
}: {
  entries: ChatEntry[];
  labels: Labels;
  children?: ReactNode;
  initialOffset?: number;
  onScroll?: (offset: number, atBottom: boolean) => void;
}) {
  const rows = ordered(entries);
  const scroller = useRef<HTMLDivElement>(null);
  // Read by the effect below without re-running it on every scroll.
  const follow = useRef(initialOffset === undefined);
  const [atBottom, setAtBottom] = useState(follow.current);
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 64,
    getItemKey: (i) => (rows[i] as ChatEntry).id,
    overscan: 6,
    // A prompt scrolled to shows below the prompt bar, not under it.
    scrollPaddingStart: 32,
    // The virtualizer scrolls there when it mounts.
    initialOffset: initialOffset ?? 0,
  });
  useEffect(() => {
    if (follow.current && rows.length > 0) virtual.scrollToIndex(rows.length - 1, { align: "end" });
  }, [rows.length, virtual]);
  const items = virtual.getVirtualItems();
  // The last prompt at or above the view's top row; rows off-screen are found by index.
  const top = virtual.range?.startIndex ?? -1;
  const prompt = atBottom
    ? undefined
    : rows
        .slice(0, top + 1)
        .reverse()
        .find((entry) => entry.kind === "user" && entry.parent === null);
  return (
    <div className="conversation">
      <div
        className="transcript hive-scroll"
        ref={scroller}
        onScroll={(event) => {
          const el = event.currentTarget;
          const bottom = el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM;
          follow.current = bottom;
          setAtBottom(bottom);
          onScroll?.(el.scrollTop, bottom);
        }}
      >
        {children}
        <ol style={{ height: virtual.getTotalSize(), position: "relative" }}>
          {items.map((item) => {
            const entry = rows[item.index] as ChatEntry;
            return (
              <li
                key={item.key}
                ref={virtual.measureElement}
                data-index={item.index}
                className="transcript-entry"
                data-role={entry.kind}
                data-status={entry.status ?? undefined}
                data-nested={entry.parent !== null || undefined}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <Row entry={entry} labels={labels} />
              </li>
            );
          })}
        </ol>
      </div>
      {prompt && (
        <button
          type="button"
          className="conversation-prompt"
          title="Scroll to this message"
          onClick={() => virtual.scrollToIndex(rows.indexOf(prompt), { align: "start" })}
        >
          <span className="transcript-role">{labels.user}</span>
          <span className="conversation-prompt-text">{prompt.text}</span>
        </button>
      )}
      {!atBottom && (
        <button
          type="button"
          className="conversation-bottom"
          title="Scroll to the bottom"
          onClick={() =>
            virtual.scrollToIndex(rows.length - 1, { align: "end", behavior: "smooth" })
          }
        >
          <ArrowDownIcon {...ICON} />
        </button>
      )}
    </div>
  );
}
