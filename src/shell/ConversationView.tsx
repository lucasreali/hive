import { ArrowDownIcon } from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { type ReactNode, useEffect, useRef, useState } from "react";
import type { TranscriptEntry } from "../store";
import { ICON } from "./icons";
import { Markdown } from "./Markdown";

/** How a subagent's conversation names its authors (6.10). */
const LABELS = { user: "Prompt", assistant: "Subagent" };

/** One entry, by its role: the subagent's messages as Markdown (8.6), the rest plain text. */
function Row({ entry }: { entry: TranscriptEntry }) {
  if (entry.role === "tool")
    return (
      <>
        <span className="transcript-role">{entry.tool}</span>
        <span className="transcript-text">{entry.text}</span>
      </>
    );
  return (
    <>
      <span className="transcript-role">{LABELS[entry.role]}</span>
      <span className="transcript-text">
        {entry.role === "assistant" ? <Markdown text={entry.text} /> : entry.text}
      </span>
    </>
  );
}

/** How close to the bottom (px, about one line) a view still counts as at the bottom (8.13). */
const AT_BOTTOM = 24;

/**
 * A subagent's conversation (6.10), newest last, virtualized; the list keeps to the bottom while
 * new entries arrive. `children` show above the entries (hints). Scrolled up (8.13), it stays put
 * and shows a "back to bottom" button and a bar with the prompt the view's top belongs to.
 */
export function ConversationView({
  entries,
  children,
}: {
  entries: TranscriptEntry[];
  children?: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  // Read by the effect below without re-running it on every scroll.
  const follow = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const virtual = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 64,
    overscan: 6,
    // A prompt scrolled to shows below the prompt bar, not under it.
    scrollPaddingStart: 32,
  });
  useEffect(() => {
    if (follow.current && entries.length > 0)
      virtual.scrollToIndex(entries.length - 1, { align: "end" });
  }, [entries.length, virtual]);
  const items = virtual.getVirtualItems();
  // The last prompt at or above the view's top row; rows off-screen are found by index.
  const top = virtual.range?.startIndex ?? -1;
  const prompt = atBottom
    ? -1
    : entries
        .slice(0, top + 1)
        .map((entry) => entry.role)
        .lastIndexOf("user");
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
        }}
      >
        {children}
        <ol style={{ height: virtual.getTotalSize(), position: "relative" }}>
          {items.map((item) => {
            const entry = entries[item.index] as TranscriptEntry;
            return (
              <li
                key={item.key}
                ref={virtual.measureElement}
                data-index={item.index}
                className="transcript-entry"
                data-role={entry.role}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <Row entry={entry} />
              </li>
            );
          })}
        </ol>
      </div>
      {prompt >= 0 && (
        <button
          type="button"
          className="conversation-prompt"
          title="Scroll to this message"
          onClick={() => virtual.scrollToIndex(prompt, { align: "start" })}
        >
          <span className="transcript-role">{LABELS.user}</span>
          <span className="conversation-prompt-text">{entries[prompt]?.text}</span>
        </button>
      )}
      {!atBottom && (
        <button
          type="button"
          className="conversation-bottom"
          title="Scroll to the bottom"
          onClick={() =>
            virtual.scrollToIndex(entries.length - 1, { align: "end", behavior: "smooth" })
          }
        >
          <ArrowDownIcon {...ICON} />
        </button>
      )}
    </div>
  );
}
