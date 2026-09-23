import { Channel, invoke } from "@tauri-apps/api/core";
import type { ServiceMessage } from "../store";
import type { Transport } from ".";

// Commands live in src-tauri/src/lib.rs (`commands`); each terminal gets its own Channel (#24).
export const tauriTransport: Transport = {
  async connect(onMessage) {
    await invoke("connect", { onMessage: new Channel<ServiceMessage>(onMessage) });
  },
  openTerminal(cwd, cols, rows, onData) {
    const channel = new Channel<ArrayBuffer>((bytes) => onData(new Uint8Array(bytes)));
    return invoke<number>("open_terminal", { cwd, cols, rows, onData: channel });
  },
  writeTerminal: (id, data) => invoke("write_terminal", { id, data }),
  resizeTerminal: (id, cols, rows) => invoke("resize_terminal", { id, cols, rows }),
  closeTerminal: (id) => invoke("close_terminal", { id }),
  listProjects: () => invoke("list_projects"),
  addProject: (path) => invoke("add_project", { path }),
};
