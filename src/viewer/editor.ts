import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { unifiedMergeView } from "@codemirror/merge";
import { Compartment, EditorState, type Extension, Prec } from "@codemirror/state";
import { oneDark } from "@codemirror/theme-one-dark";
import { EditorView, lineNumbers } from "@codemirror/view";
import type { Lines } from "../store";

/** What the viewer shows: `content`, and as a unified diff against `original` when given. */
export type Doc = { content: string; original: string | null };

/** The prototype's look (screen 1g) over One Dark (#23): its surfaces and diff colors. */
const theme = Prec.highest(
  EditorView.theme(
    {
      "&": { height: "100%", fontSize: "12px", backgroundColor: "var(--bg)" },
      "&.cm-focused": { outline: "none" },
      ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "18px" },
      ".cm-content": { padding: "4px 0" },
      ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--text-4)", border: "none" },
      ".cm-lineNumbers .cm-gutterElement": { minWidth: "32px", padding: "0 6px 0 4px" },
      "&.cm-merge-b .cm-changedLine": { backgroundColor: "rgba(161, 193, 129, 0.12)" },
      "&.cm-merge-b .cm-changedText": { background: "rgba(161, 193, 129, 0.25)" },
      ".cm-deletedChunk": { backgroundColor: "rgba(208, 114, 119, 0.12)" },
      ".cm-deletedChunk .cm-deletedText": { background: "rgba(208, 114, 119, 0.25)" },
      "&.cm-merge-b .cm-changedLineGutter": { background: "var(--state-idle)" },
      ".cm-deletedLineGutter": { background: "var(--state-error)" },
      ".cm-collapsedLines": {
        color: "var(--accent)",
        background: "rgba(116, 173, 232, 0.08)",
        fontFamily: "var(--font-sans)",
        fontSize: "var(--fs-meta)",
        // The default "⦚" marks are not in IBM Plex.
        "&:before, &:after": { display: "none" },
      },
    },
    { dark: true },
  ),
);

/**
 * The lines the main selection covers, or null when it is empty. In a diff the document is the
 * new file (removed lines are widgets, not selectable text), so these are new-file lines. A
 * selection ending at the start of a line does not take that line.
 */
export function selectedLines(state: EditorState): Lines | null {
  const { from, to } = state.selection.main;
  if (from === to) return null;
  const first = state.doc.lineAt(from).number;
  const end = state.doc.lineAt(to);
  return { from: first, to: end.from === to ? end.number - 1 : end.number };
}

/** A read-only CodeMirror view for one open file; `show` replaces what it shows. */
export type Viewer = { view: EditorView; show(doc: Doc): void; destroy(): void };

/**
 * Creates the view in `parent` for the file at `path`, highlighted by its name's language
 * (loaded on demand). CodeMirror lives outside React (#30): the component only calls `show`
 * with each new answer and `destroy` when the file closes. `show` keeps the scroll position.
 * `onSelect` hears the selected lines (null when none) whenever they may have changed.
 */
export function createViewer(
  parent: HTMLElement,
  path: string,
  onSelect: (lines: Lines | null) => void = () => {},
): Viewer {
  const language = new Compartment();
  let support: Extension = [];
  let destroyed = false;
  const view = new EditorView({ parent });
  const found = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
  void found?.load().then((loaded) => {
    support = loaded;
    if (!destroyed) view.dispatch({ effects: language.reconfigure(loaded) });
  });
  return {
    view,
    show({ content, original }) {
      const diff =
        original === null
          ? []
          : unifiedMergeView({
              original,
              mergeControls: false,
              collapseUnchanged: { margin: 3, minSize: 6 },
            });
      const top = view.scrollDOM.scrollTop;
      view.setState(
        EditorState.create({
          doc: content,
          extensions: [
            EditorState.readOnly.of(true),
            lineNumbers(),
            oneDark,
            theme,
            language.of(support),
            diff,
            EditorView.updateListener.of((u) => u.selectionSet && onSelect(selectedLines(u.state))),
          ],
        }),
      );
      view.scrollDOM.scrollTop = top;
      onSelect(null);
    },
    destroy() {
      destroyed = true;
      view.destroy();
    },
  };
}
