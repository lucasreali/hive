import { CheckIcon, CircleNotchIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { type ReactNode, useEffect, useRef } from "react";
import type { ChatEntry, ToolStatus } from "../store";

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

/** One entry, by its kind (plain text: no Markdown, 7.3 decision). */
function Row({ entry, labels }: { entry: ChatEntry; labels: Labels }) {
  const names = entry.parent === null ? labels : SUBAGENT_LABELS;
  switch (entry.kind) {
    case "user":
    case "assistant":
      return (
        <>
          <span className="transcript-role">{names[entry.kind]}</span>
          <span className="transcript-text">{entry.text}</span>
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
}

/**
 * A conversation's entries (6.10's subagent view and the chat, 7.3), newest last, virtualized;
 * the list keeps to the bottom while new ones arrive. `children` show above the entries (hints).
 */
export function ConversationView({
  entries,
  labels,
  children,
}: {
  entries: ChatEntry[];
  labels: Labels;
  children?: ReactNode;
}) {
  const rows = ordered(entries);
  const scroller = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 64,
    getItemKey: (i) => (rows[i] as ChatEntry).id,
    overscan: 6,
  });
  useEffect(() => {
    if (rows.length > 0) virtual.scrollToIndex(rows.length - 1, { align: "end" });
  }, [rows.length, virtual]);
  return (
    <div className="transcript hive-scroll" ref={scroller}>
      {children}
      <ol style={{ height: virtual.getTotalSize(), position: "relative" }}>
        {virtual.getVirtualItems().map((item) => {
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
  );
}
