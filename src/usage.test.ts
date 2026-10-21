import { expect, test } from "bun:test";
import { clock, dayClock, usageAlertText, usageLines } from "./usage";

/** Unix seconds of a local date (2026-09-28 is a Monday). */
const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 8, day, hour, minute).getTime() / 1000;

test("times read in local time, the week's with its day", () => {
  expect(clock(new Date(2026, 8, 28, 9, 5))).toBe("09:05");
  expect(dayClock(at(28, 14, 30))).toBe("Mon 14:30");
  expect(dayClock(at(4, 23, 59))).toBe("Fri 23:59");
});

test("the tooltip adds each window's pace only when the service forecasts a limit (17.23)", () => {
  const usage = { used_percentage: 42, resets_at: at(28, 14, 30) };
  const week = { used_percentage: 41, resets_at: at(28, 9) };
  const none = { session: null, week: null };
  // Lasting until their resets: as before.
  expect(usageLines(usage, null, none)).toEqual(["Session 42% · resets 14:30"]);
  expect(usageLines(usage, week, none)).toEqual([
    "Session 42% · resets 14:30",
    "Week 41% · resets Mon 09:00",
  ]);
  // A limit before the reset: under its window's line.
  expect(usageLines(usage, week, { session: at(28, 13, 50), week: at(27, 18) })).toEqual([
    "Session 42% · resets 14:30",
    "At this pace: limit at 13:50",
    "Week 41% · resets Mon 09:00",
    "At this pace: limit Sun 18:00",
  ]);
  expect(usageLines(usage, week, { session: null, week: at(27, 18) })).toEqual([
    "Session 42% · resets 14:30",
    "Week 41% · resets Mon 09:00",
    "At this pace: limit Sun 18:00",
  ]);
  // No week, no week pace.
  expect(usageLines(usage, null, { session: at(28, 13, 50), week: at(27, 18) })).toEqual([
    "Session 42% · resets 14:30",
    "At this pace: limit at 13:50",
  ]);
});

test("a usage alert names its window and threshold, and when it resets (17.23)", () => {
  const session = { type: "usage_alert", window: "session", threshold: 80 } as const;
  expect(usageAlertText({ ...session, resets_at: at(28, 14, 30) })).toEqual({
    title: "Session usage at 80%",
    body: "Resets 14:30",
  });
  const week = { type: "usage_alert", window: "week", threshold: 95 } as const;
  expect(usageAlertText({ ...week, resets_at: at(28, 9) })).toEqual({
    title: "Weekly usage at 95%",
    body: "Resets Mon 09:00",
  });
});
