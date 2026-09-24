import { Text } from "@codemirror/state";
import type { FileText, OpenFile } from "../store";

// The open file's edit buffer (3.5, #31): what the editor holds against what is on disk. Pure
// UI state; the service owns the check that makes a save safe (`save_file` with `version`).

/** A newer text on disk (`content` null: the file is gone) and its version. */
export type Disk = { content: string | null; version: string | null };

export type EditBuffer = {
  worktree: string;
  path: string;
  /** The text being edited. */
  doc: Text;
  /** The text on disk at `version`, as far as the buffer knows; null when there is no file. */
  saved: Text | null;
  version: string | null;
  /** A newer text on disk that arrived while `doc` had unsaved edits (the conflict banner). */
  conflict: Disk | null;
  /** The text sent in `save_file`, until the service answers. */
  saving: Text | null;
  /** Why the last save failed, shown as is. */
  error: string | null;
  /** Bumped when a save hit a newer version on disk, so the file is asked for again. */
  recheck: number;
};

/** A text split on "\n" only, as the editor does, so "\r" survives a save unchanged. */
export const toText = (content: string) => Text.of(content.split("\n"));

/** Unsaved edits: the text differs from the one on disk (any text when there is no file). */
export const isDirty = (b: EditBuffer) => (b.saved ? !b.doc.eq(b.saved) : b.doc.length > 0);

/** A buffer for the answer, or null when it has no text to edit (gone, binary, too large). */
export function startEdit(file: FileText): EditBuffer | null {
  if (file.content === null || file.binary || file.too_large) return null;
  const doc = toText(file.content);
  const { worktree, path, version } = file;
  const at = { saved: doc, conflict: null, saving: null, error: null, recheck: 0 };
  return { worktree, path, doc, version, ...at };
}

/**
 * A new answer for the file: text equal to the buffer's makes it clean at that version, text
 * equal to what it was based on changes nothing, a clean buffer reloads, and a dirty one keeps
 * its edits and shows the conflict. Binary or too large answers carry no text to compare.
 */
export function fromDisk(b: EditBuffer, file: FileText): EditBuffer {
  if (file.binary || file.too_large) return b;
  const disk = file.content === null ? null : toText(file.content);
  const at = { saved: disk, version: file.version, conflict: null };
  if (disk?.eq(b.doc)) return { ...b, ...at };
  if (disk && b.saved ? disk.eq(b.saved) : disk === b.saved) {
    return { ...b, version: file.version, conflict: null };
  }
  if (!isDirty(b)) return { ...b, ...at, doc: disk ?? Text.empty };
  return { ...b, conflict: { content: file.content, version: file.version } };
}

/** The buffer sending its text, or null when there is nothing to save or a save is pending. */
export function startSave(b: EditBuffer): EditBuffer | null {
  if (b.saving || !isDirty(b)) return null;
  return { ...b, saving: b.doc, error: null };
}

/** The service wrote what was sent: that text is on disk at `version`. */
export function saved(b: EditBuffer, version: string): EditBuffer {
  if (!b.saving) return b;
  return { ...b, saved: b.saving, version, saving: null };
}

/** Nothing was written; a conflict asks for the file again, which shows the banner. */
export function failed(b: EditBuffer, error: string, message: string): EditBuffer {
  const recheck = b.recheck + (error === "conflict" ? 1 : 0);
  return { ...b, saving: null, error: message, recheck };
}

/**
 * The conflict banner's choice: "Reload" takes the text on disk, "Keep mine" keeps the edits
 * against it, so the next save overwrites it.
 */
export function resolve(b: EditBuffer, keepMine: boolean): EditBuffer {
  if (!b.conflict) return b;
  const { content, version } = b.conflict;
  const disk = content === null ? null : toText(content);
  const doc = keepMine ? b.doc : (disk ?? Text.empty);
  return { ...b, doc, saved: disk, version, conflict: null, error: null };
}

/** Both name the same file. */
export const isFor = (a: OpenFile, b: OpenFile) => a.worktree === b.worktree && a.path === b.path;
