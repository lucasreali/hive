import { ListChecksIcon, QuestionIcon, ShieldWarningIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import type { ChatAnswer, ChatRequest } from "../store";
import { transport } from "../transport";
import { ICON } from "./icons";

/** Each kind's title and icon (the agent-state icons of `docs/ui-reference.md`). */
const KINDS = {
  permission: { title: "Permission request", Icon: ShieldWarningIcon },
  question: { title: "Question", Icon: QuestionIcon },
  plan: { title: "Plan approval", Icon: ListChecksIcon },
} as const;

/**
 * A permission, question or plan waiting on the human (7.3), pinned above the composer until
 * the service sends `chat_request_gone`. It takes the focus when it appears; Enter answers with
 * the primary action (in the message or feedback field: deny with it, or keep planning), Esc
 * denies. The service validates the answer; the card only sends it, once.
 */
export function ChatRequestCard({ chat, request }: { chat: number; request: ChatRequest }) {
  const [sent, setSent] = useState(false);
  /** The deny message (permission) or the feedback (plan). */
  const [note, setNote] = useState("");
  const [picked, setPicked] = useState<string[][]>(() => request.questions.map(() => []));
  const [other, setOther] = useState<string[]>(() => request.questions.map(() => ""));
  const card = useRef<HTMLElement>(null);
  useEffect(() => card.current?.focus(), []);

  const answer = (value: ChatAnswer) => {
    if (sent) return;
    setSent(true);
    void transport.chatAnswer(chat, request.id, value);
  };
  const deny = () => answer({ kind: "deny", message: note.trim() || null });
  const keepPlanning = () => answer({ kind: "keep_planning", feedback: note.trim() });
  // Per question: the free text when there is one, else the chosen labels.
  const answers = request.questions.map((_, i) =>
    other[i].trim() ? [other[i].trim()] : picked[i],
  );
  const complete = answers.every((a) => a.length > 0);
  const primary = () => {
    if (request.kind === "permission") answer({ kind: "allow" });
    else if (request.kind === "plan") answer({ kind: "approve_plan", accept_edits: false });
    else if (complete) answer({ kind: "answers", answers });
  };

  const keys = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      return deny();
    }
    const target = event.target as HTMLElement;
    // A focused button answers Enter itself (it clicks).
    if (event.key !== "Enter" || event.shiftKey || target instanceof HTMLButtonElement) return;
    if (event.nativeEvent.isComposing) return;
    event.preventDefault();
    if (target.dataset.note === undefined || !note.trim()) primary();
    else if (request.kind === "plan") keepPlanning();
    else deny();
  };

  const toggle = (i: number, label: string, on: boolean) =>
    setPicked((all) =>
      all.map((p, at) => (at !== i ? p : on ? [...p, label] : p.filter((l) => l !== label))),
    );
  const choose = (i: number, label: string) =>
    setPicked((all) => all.map((p, at) => (at === i ? [label] : p)));
  const write = (i: number, text: string) =>
    setOther((all) => all.map((o, at) => (at === i ? text : o)));

  const { title, Icon } = KINDS[request.kind];
  const noteField = (label: string, placeholder: string) => (
    <input
      type="text"
      data-note=""
      aria-label={label}
      placeholder={placeholder}
      value={note}
      disabled={sent}
      onChange={(event) => setNote(event.target.value)}
    />
  );
  return (
    <section
      ref={card}
      className="chat-card"
      data-kind={request.kind}
      aria-label={title}
      tabIndex={-1}
      onKeyDown={keys}
    >
      <header>
        <Icon {...ICON} />
        {title}
        {request.kind === "permission" && <span className="chat-card-tool">{request.tool}</span>}
      </header>
      {request.kind === "permission" && (
        <>
          <pre className="chat-card-text">{request.detail}</pre>
          {request.reason && <p className="chat-card-reason">{request.reason}</p>}
          {noteField("Deny message", "Message for Claude when denying (optional)")}
          <footer>
            <button type="button" className="secondary" disabled={sent} onClick={deny}>
              Deny <kbd>Esc</kbd>
            </button>
            <button type="button" className="primary" disabled={sent} onClick={primary}>
              Allow <kbd>Enter</kbd>
            </button>
          </footer>
        </>
      )}
      {request.kind === "question" && (
        <>
          {request.questions.map((q, i) => (
            <fieldset key={q.question} className="chat-question" disabled={sent}>
              <legend>{q.header}</legend>
              <p>{q.question}</p>
              <div className="chat-options">
                {q.options.map((o) =>
                  q.multi ? (
                    <label key={o.label} className="checkbox" title={o.description}>
                      <input
                        type="checkbox"
                        checked={picked[i].includes(o.label)}
                        onChange={(event) => toggle(i, o.label, event.target.checked)}
                      />
                      {o.label}
                    </label>
                  ) : (
                    <button
                      key={o.label}
                      type="button"
                      className="secondary"
                      title={o.description}
                      aria-pressed={picked[i][0] === o.label}
                      onClick={() => choose(i, o.label)}
                    >
                      {o.label}
                    </button>
                  ),
                )}
              </div>
              <input
                type="text"
                aria-label={`Other answer to ${q.header}`}
                placeholder="Other…"
                value={other[i]}
                onChange={(event) => write(i, event.target.value)}
              />
            </fieldset>
          ))}
          <footer>
            <button type="button" className="secondary" disabled={sent} onClick={deny}>
              Dismiss <kbd>Esc</kbd>
            </button>
            <button
              type="button"
              className="primary"
              disabled={sent || !complete}
              onClick={primary}
            >
              Send <kbd>Enter</kbd>
            </button>
          </footer>
        </>
      )}
      {request.kind === "plan" && (
        <>
          <pre className="chat-card-text">{request.plan}</pre>
          {noteField("Feedback", "What to change in the plan")}
          <footer>
            <button
              type="button"
              className="secondary"
              disabled={sent || !note.trim()}
              onClick={keepPlanning}
            >
              Keep planning
            </button>
            <button
              type="button"
              className="secondary"
              disabled={sent}
              onClick={() => answer({ kind: "approve_plan", accept_edits: true })}
            >
              Approve and accept edits
            </button>
            <button type="button" className="primary" disabled={sent} onClick={primary}>
              Approve <kbd>Enter</kbd>
            </button>
          </footer>
        </>
      )}
    </section>
  );
}
