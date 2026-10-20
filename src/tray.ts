import { attention } from "./attention";
import { spaceName } from "./notify";
import type { AgentState, SessionWindow } from "./protocol";
import { type HiveState, useHive } from "./store";
import { transport } from "./transport";
import { isMac } from "./window";

// The menu bar icon (17.22): the UI sends what the service told it (states, pending agents,
// usage) and the app side builds the icon and its menu (`src-tauri/src/tray.rs`, #37).

/** A pending agent for the menu: id, title (else "Claude"), state, space (when several). */
export type TrayAgent = [id: string, title: string, state: AgentState, space: string | null];

/** What the tray shows; `shown` false removes it (the `notifications.menu_bar` setting). */
export type Tray = {
  shown: boolean;
  /** The most urgent state among every agent, and whether it is pending (a seen 🟠 is not). */
  urgent: [AgentState, boolean] | null;
  agents: TrayAgent[];
  /** The usage ring's lines. */
  usage: string[];
};

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** "14:30" in local time. */
function clock(at: Date) {
  return at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

/**
 * The usage ring's text (15.3): "Session 42% · resets 14:30" and, when the service sent one,
 * "Week 41% · resets Mon 09:00"; none without a session window.
 */
export function usageLines(usage: SessionWindow | null, week: SessionWindow | null): string[] {
  if (!usage) return [];
  const lines = [
    `Session ${usage.used_percentage}% · resets ${clock(new Date(usage.resets_at * 1000))}`,
  ];
  if (week) {
    const at = new Date(week.resets_at * 1000);
    lines.push(`Week ${week.used_percentage}% · resets ${WEEKDAYS[at.getDay()]} ${clock(at)}`);
  }
  return lines;
}

/** The tray for the store: `attention`'s pending agents and most urgent state, and the usage. */
export function trayOf(s: HiveState): Tray {
  const { pending, urgent } = attention(s);
  return {
    shown: s.settings.notifications.menu_bar,
    urgent: urgent && [urgent.state, urgent.pending],
    agents: pending.map((a) => [
      a.id,
      s.agentTitles[a.id] ?? "Claude",
      s.agentStates[a.id]?.state ?? "idle",
      spaceName(s, a.id) ?? null,
    ]),
    usage: usageLines(s.sessionUsage, s.weekUsage),
  };
}

/**
 * Keeps the menu bar icon on the store, sent only when it changes. Only on macOS (the menu bar;
 * no Windows-only work, D17.0). Returns the unsubscribe.
 */
export function followTray(
  send: (tray: Tray) => Promise<void> = (tray) => transport.setTray(tray),
  mac = isMac(),
): () => void {
  if (!mac) return () => {};
  let sent = "";
  const sync = (s: HiveState) => {
    const tray = trayOf(s);
    const json = JSON.stringify(tray);
    if (json === sent) return;
    sent = json;
    // A failure is dropped: the icon is a hint.
    send(tray).catch(() => {});
  };
  sync(useHive.getState());
  return useHive.subscribe(sync);
}
