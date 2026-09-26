import { addTab, type Chat, type ChatMode, removeTab, setChat, setNotice, useHive } from "./store";
import { transport } from "./transport";

// Chats (7.3): a tab of the terminal area whose Claude runs headless in the service. The tab bar's
// "+" menu's "Agent" (7.5) calls `openChat`.

/**
 * Starts a chat in the worktree `cwd` (going on with session `resume`, whose history the service
 * sends first) and adds its tab, shown. The service may first ask to confirm the folder
 * (`confirm_chat_folder`), which the chat's view shows.
 */
export async function openChat(
  cwd: string,
  resume: string | null = null,
  mode: ChatMode | null = null,
): Promise<number> {
  const id = await transport.openChat(cwd, resume, mode);
  setChat(id, cwd);
  addTab(id, cwd, "chat");
  return id;
}

/** Ends the chat (unless it already ended) and drops its tab and its entries. */
export function closeChat(id: number): void {
  if (!useHive.getState().chats[id]?.closed) void transport.closeChat(id);
  setChat(id, null);
  removeTab(id);
}

/** The chat's Claude session, once the service named it. */
export const chatSession = (chat: Chat | undefined) =>
  chat?.status?.session ?? chat?.opened?.session ?? null;

/** Goes on with an ended chat's session in a new chat, in the same mode, which replaces its tab. */
export async function resumeChat(id: number): Promise<void> {
  const chat = useHive.getState().chats[id];
  const session = chatSession(chat);
  if (!chat || !session) return;
  try {
    await openChat(chat.cwd, session, chat.status?.mode ?? chat.opened?.mode ?? null);
    closeChat(id);
  } catch (error) {
    setNotice(`Cannot resume the chat in ${chat.cwd}: ${error}`);
  }
}
