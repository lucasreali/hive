import { PaperPlaneRightIcon, StopIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, useState } from "react";
import { useHive } from "../store";
import { transport } from "../transport";
import { ICON } from "./icons";

/**
 * A chat's input (7.3): Enter sends, Shift+Enter starts a new line. While a turn runs, Send
 * turns into Stop; closed (or not started yet) it is disabled. The draft is UI state.
 */
export function ChatComposer({ chat }: { chat: number }) {
  const [text, setText] = useState("");
  const busy = useHive((s) => !!s.chats[chat]?.status?.busy);
  const ready = useHive((s) => !!s.chats[chat]?.opened && !s.chats[chat]?.closed);
  const send = () => {
    if (!ready || busy || text.trim() === "") return;
    void transport.chatSend(chat, text, []);
    setText("");
  };
  const keys = (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    send();
  };
  return (
    <form
      className="chat-composer"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <textarea
        aria-label="Message"
        placeholder={ready ? "Message Claude (Shift+Enter for a new line)" : undefined}
        rows={3}
        value={text}
        disabled={!ready}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={keys}
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
        <button type="submit" className="primary" disabled={!ready || text.trim() === ""}>
          <PaperPlaneRightIcon {...ICON} /> Send
        </button>
      )}
    </form>
  );
}
