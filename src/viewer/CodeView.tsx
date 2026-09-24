import { useEffect, useRef } from "react";
import { clearGotoLine, type FileText, setSelectedLines, useHive } from "../store";
import { createViewer, revealLine, type Viewer } from "./editor";

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
  const path = text.path;
  useEffect(() => {
    const created = createViewer(parent.current as HTMLDivElement, path, setSelectedLines);
    viewer.current = created;
    return () => {
      created.destroy();
      setSelectedLines(null);
    };
  }, [path]);
  useEffect(() => {
    const original = diff ? (text.base ?? "") : null;
    viewer.current?.show({ content: text.content ?? "", original });
  }, [text, diff]);
  // A search result asked for a line: shown once this file's text is there.
  const goto = useHive((s) => s.gotoLine);
  useEffect(() => {
    const view = viewer.current?.view;
    if (!view || !goto || goto.path !== path || goto.worktree !== text.worktree) return;
    revealLine(view, goto.line);
    clearGotoLine();
  }, [goto, text, path]);
  return <div className="code-view" ref={parent} />;
}
