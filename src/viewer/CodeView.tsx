import { useEffect, useRef } from "react";
import { clearGotoLine, type FileText, saveFileView, setSelectedLines, useHive } from "../store";
import { isFor } from "./buffer";
import { createViewer, restoreView, revealLine, snapshot, type Viewer } from "./editor";

/** Why the service sent no text to show, or null. */
export function notice(text: FileText): string | null {
  if (text.error) return text.error;
  if (text.binary) return "Binary file not shown.";
  if (text.too_large) return "File too large to show.";
  return null;
}

/**
 * The open file in CodeMirror, read-only: a unified diff against HEAD when `diff` (a deleted
 * file shows all removed, a new one all added), else its text. Key it by the file, so the
 * view is created once per open file and only updated with each new answer.
 */
export function CodeView({ text, diff }: { text: FileText; diff: boolean }) {
  const parent = useRef<HTMLDivElement>(null);
  const viewer = useRef<Viewer | null>(null);
  // The view as its tab left it (8.21), put back once the text first shows.
  const saved = useRef<unknown>(null);
  const { path, worktree } = text;
  useEffect(() => {
    const file = { worktree, path };
    const created = createViewer(parent.current as HTMLDivElement, path, setSelectedLines);
    viewer.current = created;
    saved.current = useHive.getState().openFiles.find((f) => isFor(f, file))?.view;
    return () => {
      saveFileView(file, snapshot(created.view));
      created.destroy();
      setSelectedLines(null);
    };
  }, [path, worktree]);
  useEffect(() => {
    const original = diff ? (text.base ?? "") : null;
    const view = viewer.current as Viewer;
    view.show({ content: text.content ?? "", original });
    restoreView(view.view, saved.current);
    saved.current = null;
  }, [text, diff]);
  // A search result asked for a line: shown once this file's text is there.
  const goto = useHive((s) => s.gotoLine);
  useEffect(() => {
    const view = viewer.current?.view;
    if (!view || !goto || goto.path !== path || goto.worktree !== text.worktree) return;
    revealLine(view, goto.line);
    clearGotoLine();
  }, [goto, text, path]);
  // Review comments on this file (6.7), marked on their lines.
  const comments = useHive((s) => s.comments[text.worktree]);
  useEffect(() => {
    viewer.current?.mark((comments ?? []).filter((c) => c.path === path));
  }, [comments, path]);
  return <div className="code-view" ref={parent} />;
}
