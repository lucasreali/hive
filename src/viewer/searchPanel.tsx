import {
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  replaceAll,
  replaceNext,
  SearchQuery,
  setSearchQuery,
} from "@codemirror/search";
import { type EditorView, type Panel, runScopeHandlers, type ViewUpdate } from "@codemirror/view";
import { ArrowDownIcon, ArrowUpIcon, XIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { ICON } from "../shell/icons";
import { composing, keyText } from "../window";

// 23.10: the editor's find bar, CodeMirror's search panel drawn with the app's fields and
// buttons. Only the look is ours: every control runs CodeMirror's own command, and its keymap
// (`editorSearchKeymap`) still runs inside the bar, as in the stock panel. The table view's bar
// (24.12) is the same `FindBar` over its own matches.

type Option = "caseSensitive" | "regexp" | "wholeWord";

/** The field's toggles: the text they show and what they are called. */
const OPTIONS: { option: Option; text: string; label: string }[] = [
  { option: "caseSensitive", text: "Aa", label: "Match case" },
  { option: "regexp", text: ".*", label: "Use regular expression" },
  { option: "wholeWord", text: "ab", label: "Match whole word" },
];

/** `query` with `change` applied, as CodeMirror's panel builds it (every field it reads). */
function changed(
  query: SearchQuery,
  change: Partial<Record<Option, boolean>> & Partial<Record<"search" | "replace", string>>,
): SearchQuery {
  const { search, caseSensitive, literal, regexp, wholeWord, replace } = query;
  return new SearchQuery({ search, caseSensitive, literal, regexp, wholeWord, replace, ...change });
}

/** Whether `query` is a regular expression that does not compile (24.12). */
export const badRegExp = (query: SearchQuery) => query.regexp && !!query.search && !query.valid;

export interface FindBarProps {
  query: SearchQuery;
  /** A new query, from the field or a toggle. */
  onQuery: (query: SearchQuery) => void;
  onNext: () => void;
  onPrevious: () => void;
  onClose: () => void;
  /** The replace row's actions, only where the text can change. */
  replace?: { next: () => void; all: () => void };
  /** What the bar says after the field (the table's count), unless the regexp is bad. */
  status?: string;
}

/**
 * The find bar's rows, shared by the editor's panel and the table view (24.12): the field with
 * its toggles, previous / next, close and, given `replace`, the replace row. Enter in the find
 * field goes to the next match (Shift: the previous one); a regexp that does not compile marks
 * the field and says so.
 */
export function FindBar(props: FindBarProps) {
  const { query, onQuery, onNext, onPrevious, onClose, replace } = props;
  const set = (change: Parameters<typeof changed>[1]) => onQuery(changed(query, change));
  const bad = badRegExp(query);
  const status = bad ? "Invalid regular expression" : props.status;
  const button = (label: string, title: string, run: () => void, content: ReactNode) => (
    <button
      type="button"
      className="ghost"
      aria-label={label}
      title={keyText(title)}
      onClick={() => run()}
    >
      {content}
    </button>
  );
  return (
    <>
      <div className="find-row">
        <div className="find-field">
          <input
            type="text"
            name="search"
            main-field="true"
            spellCheck={false}
            autoComplete="off"
            placeholder="Find"
            aria-label="Find"
            aria-invalid={bad}
            value={query.search}
            onChange={(e) => set({ search: e.target.value })}
            onKeyDown={(e) => {
              if (e.key !== "Enter" || composing(e.nativeEvent)) return;
              e.preventDefault();
              (e.shiftKey ? onPrevious : onNext)();
            }}
          />
          {OPTIONS.map(({ option, text, label }) => (
            <button
              key={option}
              type="button"
              className="find-toggle"
              name={option}
              aria-label={label}
              title={label}
              aria-pressed={query[option]}
              onClick={() => set({ [option]: !query[option] })}
            >
              {text}
            </button>
          ))}
        </div>
        {status && (
          <span className="find-status" aria-live="polite">
            {status}
          </span>
        )}
        {button(
          "Previous match",
          "Previous match (Shift+Enter)",
          onPrevious,
          <ArrowUpIcon {...ICON} />,
        )}
        {button("Next match", "Next match (Enter)", onNext, <ArrowDownIcon {...ICON} />)}
        <span className="find-gap" />
        {button("Close search", "Close (Esc)", onClose, <XIcon {...ICON} />)}
      </div>
      {replace && (
        <div className="find-row">
          <div className="find-field">
            <input
              type="text"
              name="replace"
              spellCheck={false}
              autoComplete="off"
              placeholder="Replace"
              aria-label="Replace"
              value={query.replace}
              onChange={(e) => set({ replace: e.target.value })}
            />
          </div>
          <button
            type="button"
            className="ghost text"
            title={keyText("Replace the current match (Enter)")}
            onClick={() => replace.next()}
          >
            Replace
          </button>
          <button
            type="button"
            className="ghost text"
            title="Replace every match"
            onClick={() => replace.all()}
          >
            Replace all
          </button>
        </div>
      )}
    </>
  );
}

/** The editor's bar for `view` showing `query`; the replace row only when the text can change. */
function EditorFindBar({ view, query }: { view: EditorView; query: SearchQuery }) {
  return (
    <FindBar
      query={query}
      onQuery={(next) => view.dispatch({ effects: setSearchQuery.of(next) })}
      onNext={() => findNext(view)}
      onPrevious={() => findPrevious(view)}
      onClose={() => closeSearchPanel(view)}
      replace={
        view.state.readOnly
          ? undefined
          : { next: () => replaceNext(view), all: () => replaceAll(view) }
      }
    />
  );
}

/**
 * The search panel for `search({ createPanel })`: a React root in the panel's box, drawn again
 * when the query or the read-only state changes. Keys in the bar run the "search-panel" scope of
 * the editor's keymap (Esc, ⌘G / F3, ⌘F); Enter in the find field goes to the next match
 * (Shift: the previous one), in the replace field replaces the current one.
 */
export function createSearchPanel(view: EditorView): Panel {
  const dom = document.createElement("div");
  dom.className = "cm-search find-bar";
  dom.setAttribute("role", "search");
  dom.setAttribute("aria-label", "Find in the file");
  const root = createRoot(dom);
  let shown: { query: SearchQuery; readOnly: boolean } | null = null;
  const draw = () => {
    const query = getSearchQuery(view.state);
    const readOnly = view.state.readOnly;
    if (shown?.query.eq(query) && shown.readOnly === readOnly) return;
    shown = { query, readOnly };
    root.render(<EditorFindBar view={view} query={query} />);
  };
  // Drawn at once: CodeMirror gives its field the keyboard right after creating the panel.
  flushSync(draw);
  dom.addEventListener("keydown", (e) => {
    const field = (e.target as HTMLInputElement).name;
    if (runScopeHandlers(view, e, "search-panel")) e.preventDefault();
    else if (e.key === "Enter" && !e.isComposing && field === "replace") {
      e.preventDefault();
      replaceNext(view);
    }
  });
  return {
    dom,
    top: true,
    mount() {
      const field = dom.querySelector<HTMLInputElement>("[main-field]");
      field?.focus();
      field?.select();
    },
    update(_: ViewUpdate) {
      draw();
    },
    // Later: the bar may close from one of its own buttons, while React handles the click.
    destroy() {
      queueMicrotask(() => root.unmount());
    },
  };
}
