import { PaperPlaneRightIcon, StopIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, useId, useState } from "react";
import { type ChatMode, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { ICON } from "./icons";

/** The permission modes offered (7.3): never `bypassPermissions`. */
export const MODES: { value: ChatMode; label: string }[] = [
  { value: "default", label: "Default" },
  { value: "accept_edits", label: "Accept edits" },
  { value: "plan", label: "Plan" },
];

const NO_COMMANDS: string[] = [];

/**
 * A chat's input (7.3): Enter sends, Shift+Enter starts a new line. While a turn runs, Send
 * turns into Stop, and Esc stops too; closed (or not started yet) it is disabled. The draft is
 * UI state. Typing `/` lists the chat's slash commands that start with what follows it: ↑/↓
 * move, Enter or Tab picks, Esc hides the list. The mode selector shows the service's mode and
 * asks it for another.
 */
export function ChatComposer({ chat }: { chat: number }) {
  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  // The draft the list was hidden for (Esc); typing shows it again.
  const [hidden, setHidden] = useState<string | null>(null);
  const id = useId();
  const busy = useHive((s) => !!s.chats[chat]?.status?.busy);
  const ready = useHive((s) => !!s.chats[chat]?.opened && !s.chats[chat]?.closed);
  const mode = useHive((s) => s.chats[chat]?.status?.mode ?? s.chats[chat]?.opened?.mode);
  const commands = useHive((s) => s.chats[chat]?.opened?.commands ?? NO_COMMANDS);
  const typed = /^\/\S*$/.test(text) && text !== hidden ? text.slice(1) : null;
  const matches = typed === null ? [] : commands.filter((c) => c.startsWith(typed));
  const at = Math.min(active, matches.length - 1);
  const edit = (value: string) => {
    setText(value);
    setActive(0);
  };
  const pick = (command: string) => edit(`/${command} `);
  const send = () => {
    if (!ready || busy || text.trim() === "") return;
    void transport.chatSend(chat, text, []);
    edit("");
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
  return (
    <form
      className="chat-composer"
      onSubmit={(event) => {
        event.preventDefault();
        send();
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
      <textarea
        aria-label="Message"
        aria-controls={matches.length > 0 ? id : undefined}
        aria-activedescendant={matches.length > 0 ? `${id}-${at}` : undefined}
        placeholder={
          ready ? "Message Claude (Shift+Enter for a new line, / for commands)" : undefined
        }
        rows={3}
        value={text}
        disabled={!ready}
        onChange={(event) => edit(event.target.value)}
        onKeyDown={keys}
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
          className="secondary"
          title="Stop the turn (Esc)"
          onClick={() => void transport.chatInterrupt(chat)}
        >
          <StopIcon {...ICON} /> Stop
        </button>
      ) : (
        <button type="submit" className="primary" disabled={!ready || text.trim() === ""}>
          <PaperPlaneRightIcon {...ICON} /> Send
        </button>
      )}
    </form>
  );
}
