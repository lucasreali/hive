import { useEffect } from "react";
import { type ChatEntry, hideTranscript, type SubagentRef, useHive } from "../store";
import { transport } from "../transport";
import { ConversationView, SUBAGENT_LABELS } from "./ConversationView";
import { STATE_LABEL, StateIcon, TerminalIcon } from "./icons";

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
  const entries = (transcript?.entries ?? []).map(
    (e, id): ChatEntry => ({
      id,
      kind: e.role,
      text: e.text,
      tool: e.tool,
      parent: null,
      status: null,
      output: null,
      images: [],
    }),
  );
  useEffect(() => {
    void transport.watchTranscript(agent, subagent);
    return () => void transport.unwatchTranscript(agent, subagent);
  }, [agent, subagent]);
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
      <ConversationView entries={entries} labels={SUBAGENT_LABELS}>
        {!transcript && <div className="hint">Loading the conversation…</div>}
        {transcript && entries.length === 0 && <div className="hint">Nothing written yet.</div>}
        {transcript?.truncated && <div className="hint">Earlier messages are left out.</div>}
      </ConversationView>
    </section>
  );
}
