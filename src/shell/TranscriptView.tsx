import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef } from "react";
import { hideTranscript, type SubagentRef, type TranscriptEntry, useHive } from "../store";
import { transport } from "../transport";
import { STATE_LABEL, StateIcon, TerminalIcon } from "./icons";

const ROLE: Record<TranscriptEntry["role"], string> = {
  user: "Prompt",
  assistant: "Subagent",
  tool: "Tool",
};

/**
 * A subagent's conversation, read-only (6.10), in place of the terminals: the service follows
 * its transcript while this shows. Newest entries last; the list keeps to the bottom while new
 * ones arrive.
 */
export function TranscriptView({ agent, subagent }: SubagentRef) {
  const transcript = useHive((s) =>
    s.transcript?.agent === agent && s.transcript.subagent === subagent ? s.transcript : null,
  );
  // A subagent that left the agent's list has ended.
  const sub = useHive((s) => s.agentStates[agent]?.subagents.find((x) => x.id === subagent));
  const state = sub?.state ?? "ended";
  const entries = transcript?.entries ?? [];
  const scroller = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 64,
    overscan: 6,
  });
  useEffect(() => {
    void transport.watchTranscript(agent, subagent);
    return () => void transport.unwatchTranscript(agent, subagent);
  }, [agent, subagent]);
  useEffect(() => {
    if (entries.length > 0) virtual.scrollToIndex(entries.length - 1, { align: "end" });
  }, [entries.length, virtual]);
  return (
    <section className="file-view transcript-view" aria-label="Subagent conversation">
      <div className="file-view-bar">
        <StateIcon state={state} />
        <span className="path">
          subagent: {sub?.agent_type ?? "unknown"}
          <span className="state-label" data-state={state}>
            {STATE_LABEL[state]}
          </span>
        </span>
        <button type="button" className="ghost text" onClick={hideTranscript}>
          <TerminalIcon /> Back to terminal
        </button>
      </div>
      <div className="transcript hive-scroll" ref={scroller}>
        {!transcript && <div className="hint">Loading the conversation…</div>}
        {transcript && entries.length === 0 && <div className="hint">Nothing written yet.</div>}
        {transcript?.truncated && <div className="hint">Earlier messages are left out.</div>}
        <ol style={{ height: virtual.getTotalSize(), position: "relative" }}>
          {virtual.getVirtualItems().map((item) => {
            const entry = entries[item.index] as TranscriptEntry;
            return (
              <li
                key={item.index}
                ref={virtual.measureElement}
                data-index={item.index}
                className="transcript-entry"
                data-role={entry.role}
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <span className="transcript-role">{entry.tool ?? ROLE[entry.role]}</span>
                <span className="transcript-text">{entry.text}</span>
              </li>
            );
          })}
        </ol>
      </div>
    </section>
  );
}
