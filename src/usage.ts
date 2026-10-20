import type { ServiceMessage, SessionWindow } from "./protocol";

// The words for the usage windows the service sends (15.3) and their pace and alerts (17.23):
// the service decides every number and time; this only writes them in local time.

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** "14:30" in local time. */
export function clock(at: Date): string {
  return at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

/** "Mon 09:00" in local time, for `at` in Unix seconds. */
export function dayClock(at: number): string {
  const date = new Date(at * 1000);
  return `${WEEKDAYS[date.getDay()]} ${clock(date)}`;
}

/** "14:30" in local time, for `at` in Unix seconds. */
const time = (at: number) => clock(new Date(at * 1000));

/** When each window reaches its limit at its pace, as the store keeps it. */
export type UsageLimits = { session: number | null; week: number | null };

/**
 * The usage ring's tooltip: "Session 42% · resets 14:30", "At this pace: limit at 13:50" when
 * the service forecasts one, then the same for the week when it sent one ("Week 41% · resets
 * Mon 09:00", "At this pace: limit Sun 18:00").
 */
export function usageLines(
  usage: SessionWindow,
  week: SessionWindow | null,
  limits: UsageLimits,
): string[] {
  const lines = [`Session ${usage.used_percentage}% · resets ${time(usage.resets_at)}`];
  if (limits.session !== null) lines.push(`At this pace: limit at ${time(limits.session)}`);
  if (week) {
    lines.push(`Week ${week.used_percentage}% · resets ${dayClock(week.resets_at)}`);
    if (limits.week !== null) lines.push(`At this pace: limit ${dayClock(limits.week)}`);
  }
  return lines;
}

type UsageAlert = Extract<ServiceMessage, { type: "usage_alert" }>;

/** A usage alert's notification: "Session usage at 80%" / "Resets 14:30" (the week's: its day). */
export function usageAlertText(alert: UsageAlert): { title: string; body: string } {
  const session = alert.window === "session";
  const resets = session ? time(alert.resets_at) : dayClock(alert.resets_at);
  return {
    title: `${session ? "Session" : "Weekly"} usage at ${alert.threshold}%`,
    body: `Resets ${resets}`,
  };
}
