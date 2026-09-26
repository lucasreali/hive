import { useState } from "react";
import { type Chat, useHive } from "../store";
import { transport } from "../transport";
import { ChatComposer } from "./ChatComposer";
import { ChatRequestCard } from "./ChatRequestCard";
import { CHAT_LABELS, ConversationView } from "./ConversationView";
import { ChatIcon, CloseIcon } from "./icons";

/** The chat's state in its bar. */
function stateText(chat: Chat): string {
  if (chat.closed) return "ended";
  if (!chat.opened) return "starting";
  return chat.status?.busy ? "working" : "ready";
}

/**
 * The first chat in a project (7.3): Claude will run tools in that folder, so the service asks
 * once. Esc or Cancel refuses (the chat then closes).
 */
function FolderDialog({ chat, cwd }: { chat: number; cwd: string }) {
  const [answered, setAnswered] = useState(false);
  if (answered) return null;
  const answer = (accepted: boolean) => {
    setAnswered(true);
    void transport.confirmChatFolder(chat, cwd, accepted);
  };
  return (
    <dialog
      className="dialog"
      aria-labelledby="chat-folder-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          dialog.querySelector<HTMLButtonElement>(".primary")?.focus();
        }
      }}
      onClose={() => answer(false)}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          answer(true);
        }}
      >
        <header>
          <h2 id="chat-folder-title">Chat in this folder?</h2>
          <button
            type="button"
            className="ghost"
            title="Cancel (Esc)"
            onClick={() => answer(false)}
          >
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            This is the first chat in <code>{cwd}</code>. Claude runs in this folder with its
            project settings, and its tools read files, edit them and run commands here; it asks
            before anything that needs your permission.
          </p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={() => answer(false)}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary">
            Start chat <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}

/** A chat tab's body (7.3), in place of the terminals: its entries and the composer. */
export function ChatView({ id }: { id: number }) {
  const chat = useHive((s) => s.chats[id]);
  if (!chat) return null;
  const state = stateText(chat);
  return (
    <section className="file-view transcript-view chat-view" aria-label="Chat">
      <div className="file-view-bar">
        <ChatIcon />
        <span className="path">
          chat: {chat.cwd}
          <span className="state-label" data-state={state}>
            {state}
          </span>
        </span>
      </div>
      <ConversationView entries={chat.entries} labels={CHAT_LABELS}>
        {!chat.opened && !chat.closed && <div className="hint">Starting Claude…</div>}
        {chat.opened && chat.entries.length === 0 && (
          <div className="hint">Send a message to start.</div>
        )}
      </ConversationView>
      {chat.closed?.error && (
        <div className="files-error" role="alert">
          {chat.closed.error}
        </div>
      )}
      {chat.requests.length > 0 && (
        <div className="chat-requests">
          {chat.requests.map((request) => (
            <ChatRequestCard key={request.id} chat={id} request={request} />
          ))}
        </div>
      )}
      <ChatComposer chat={id} />
      {chat.confirm && <FolderDialog chat={id} cwd={chat.cwd} />}
    </section>
  );
}
