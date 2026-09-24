import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { unifiedMergeView } from "@codemirror/merge";
import {
  Compartment,
  EditorSelection,
  EditorState,
  type Extension,
  Prec,
  type Text,
} from "@codemirror/state";
import { oneDark } from "@codemirror/theme-one-dark";
import { drawSelection, EditorView, keymap, lineNumbers } from "@codemirror/view";
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

/**
 * The file name's language for `view`, loaded on demand: `extension()` is what to put in a new
 * state (empty until loaded), and a view still open gets it once it loads.
 */
function highlighting(view: EditorView, path: string) {
  const language = new Compartment();
  let support: Extension = [];
  let destroyed = false;
  const found = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
  void found?.load().then((loaded) => {
    support = loaded;
    if (!destroyed) view.dispatch({ effects: language.reconfigure(loaded) });
  });
  return {
    extension: () => language.of(support),
    destroy() {
      destroyed = true;
      view.destroy();
    },
  };
}

/** Replaces the view's state, keeping its scroll position. */
function replaceState(view: EditorView, state: EditorState) {
  const top = view.scrollDOM.scrollTop;
  view.setState(state);
  view.scrollDOM.scrollTop = top;
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
  const view = new EditorView({ parent });
  const language = highlighting(view, path);
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
      replaceState(
        view,
        EditorState.create({
          doc: content,
          extensions: [
            EditorState.readOnly.of(true),
            lineNumbers(),
            oneDark,
            theme,
            language.extension(),
            diff,
            EditorView.updateListener.of((u) => u.selectionSet && onSelect(selectedLines(u.state))),
          ],
        }),
      );
      onSelect(null);
    },
    destroy: language.destroy,
  };
}

/** What the editable view tells its owner. */
export type EditorEvents = {
  /** The user changed the text. */
  change(doc: Text): void;
  /** Ctrl+S, only while the editor has the focus (never the terminal's key). */
  save(): void;
  /** The selected lines (null when none), for the terminal reference (3.4). */
  select(lines: Lines | null): void;
};

/** An editable CodeMirror view for the open file's buffer. */
export type Editor = {
  view: EditorView;
  /** Shows `doc` unless it already is the view's (a reload); keeps scroll and selection. */
  load(doc: Text): void;
  /** A read-only unified diff of the text against `original` (on disk), or null to edit. */
  compare(original: string | null): void;
  destroy(): void;
};

/**
 * Creates the editable view in `parent` for the file at `path`, holding `doc`. Lines split on
 * "\n" only, so a "\r" stays in the text and saves unchanged. Standard keys, undo, and Tab
 * indents (Esc, then Tab, leaves the editor).
 */
export function createEditor(
  parent: HTMLElement,
  path: string,
  doc: Text,
  on: EditorEvents,
): Editor {
  const view = new EditorView({ parent });
  const language = highlighting(view, path);
  const comparing = new Compartment();
  const create = (doc: Text, selection?: EditorSelection) =>
    EditorState.create({
      doc,
      selection,
      extensions: [
        EditorState.lineSeparator.of("\n"),
        history(),
        keymap.of([
          {
            key: "Mod-s",
            run: () => {
              on.save();
              return true;
            },
          },
          indentWithTab,
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        drawSelection(),
        lineNumbers(),
        oneDark,
        theme,
        language.extension(),
        comparing.of([]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) on.change(u.state.doc);
          if (u.selectionSet) on.select(selectedLines(u.state));
        }),
      ],
    });
  view.setState(create(doc));
  return {
    view,
    load(doc) {
      if (doc === view.state.doc) return;
      const { ranges, mainIndex } = view.state.selection;
      const clamp = (n: number) => Math.min(n, doc.length);
      const kept = ranges.map((r) => EditorSelection.range(clamp(r.anchor), clamp(r.head)));
      const state = create(doc, EditorSelection.create(kept, mainIndex));
      replaceState(view, state);
      on.select(selectedLines(state));
    },
    compare(original) {
      const diff =
        original === null
          ? []
          : [EditorState.readOnly.of(true), unifiedMergeView({ original, mergeControls: false })];
      view.dispatch({ effects: comparing.reconfigure(diff) });
    },
    destroy: language.destroy,
  };
}
