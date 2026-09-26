import { addTab, removeTab, setChat, useHive } from "./store";
import { transport } from "./transport";

// Chats (7.3): a tab of the terminal area whose Claude runs headless in the service. The tab bar's
// "+" menu (7.5, "Agent") and the temporary "New chat" button call `openChat`.

/**
 * Starts a chat in the worktree `cwd` and adds its tab, shown. The service may first ask to
 * confirm the folder (`confirm_chat_folder`), which the chat's view shows.
 */
export async function openChat(cwd: string): Promise<number> {
  const id = await transport.openChat(cwd, null, null);
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
