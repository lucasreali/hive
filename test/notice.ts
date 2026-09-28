import { useHive } from "../src/store";

/** The newest toast's text (10.3), null when none shows. */
export const notice = () => useHive.getState().notices.at(-1)?.text ?? null;

/** The newest toast's kind: "error" (it stays) or "info" (it fades). */
export const noticeKind = () => useHive.getState().notices.at(-1)?.kind;
