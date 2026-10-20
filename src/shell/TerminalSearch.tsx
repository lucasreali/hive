import { CaretDownIcon, CaretUpIcon, TextAaIcon, XIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { closeTerminalSearch, openTerminalSearch, terminalShown, useHive } from "../store";
import {
  endSearch,
  onSearchResults,
  paneOf,
  type SearchResults,
  searchTerminal,
} from "../terminals";
import { composing } from "../window";
import { ICON } from "./icons";

// 17.21: the terminal's search bar (⌘F on macOS, Ctrl+Shift+F elsewhere), over the focused
// terminal's pane. xterm's search addon searches the scrollback and the screen; this bar only
// holds what is typed and shows what it found.

/** The most matches the search addon counts (its highlight limit). */
export const MATCH_LIMIT = 1000;

/** The bar's count: "3 of 12", "12 matches" before one is current, "No results". */
export function resultText({ index, count }: SearchResults): string {
  if (count === 0) return "No results";
  const total = count >= MATCH_LIMIT ? `${MATCH_LIMIT}+` : `${count}`;
  return index < 0 ? `${total} matches` : `${index + 1} of ${total}`;
}

const FIELD = ".terminal-search input";

/**
 * The search command: opens the bar over the terminal shown, or, already open, gives its field
 * the keyboard with the text selected, to type a new search.
 */
export function findInTerminal(): void {
  openTerminalSearch();
  const field = document.querySelector<HTMLInputElement>(FIELD);
  field?.focus();
  field?.select();
}

/**
 * The bar over terminal `id`: a search field, its count, next / previous, a case toggle. As in
 * iTerm2 and Ghostty it searches up, from the newest output: "next" is the older match (Enter),
 * "previous" the newer one (Shift+Enter).
 */
function SearchBar({ id }: { id: number }) {
  const [text, setText] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [results, setResults] = useState<SearchResults | null>(null);
  useEffect(() => onSearchResults(id, setResults), [id]);
  // Closed (or its terminal no longer shown): no match stays highlighted.
  useEffect(() => () => searchTerminal(id, "", { caseSensitive: false }), [id]);
  const find = (options: { newer?: boolean } = {}) =>
    searchTerminal(id, text, { caseSensitive, ...options });
  const close = () => {
    closeTerminalSearch();
    endSearch(id);
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: <search> is unknown to older WebKit
    <div className="terminal-search" role="search" aria-label="Search the terminal">
      <input
        // biome-ignore lint/a11y/noAutofocus: the bar opens to type a search
        autoFocus
        type="text"
        spellCheck={false}
        autoComplete="off"
        placeholder="Search"
        aria-label="Search the terminal"
        value={text}
        onChange={(e) => {
          const query = e.target.value;
          setText(query);
          if (!query) setResults(null);
          searchTerminal(id, query, { caseSensitive });
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close();
          } else if (e.key === "Enter" && !composing(e.nativeEvent) && text) {
            e.preventDefault();
            find({ newer: e.shiftKey });
          }
        }}
      />
      <span className="terminal-search-count" aria-live="polite">
        {text && results ? resultText(results) : ""}
      </span>
      <button
        type="button"
        className="ghost"
        title="Match case"
        aria-label="Match case"
        aria-pressed={caseSensitive}
        onClick={() => {
          setCaseSensitive(!caseSensitive);
          searchTerminal(id, text, { caseSensitive: !caseSensitive, restart: true });
        }}
      >
        <TextAaIcon {...ICON} />
      </button>
      <button
        type="button"
        className="ghost"
        title="Next match, older (Enter)"
        aria-label="Next match"
        disabled={!text}
        onClick={() => find()}
      >
        <CaretUpIcon {...ICON} />
      </button>
      <button
        type="button"
        className="ghost"
        title="Previous match, newer (Shift+Enter)"
        aria-label="Previous match"
        disabled={!text}
        onClick={() => find({ newer: true })}
      >
        <CaretDownIcon {...ICON} />
      </button>
      <button
        type="button"
        className="ghost"
        title="Close (Esc)"
        aria-label="Close search"
        onClick={close}
      >
        <XIcon {...ICON} />
      </button>
    </div>
  );
}

/** The search bar, inside the pane of the terminal it searches while that terminal is shown. */
export function TerminalSearch() {
  const id = useHive((s) =>
    terminalShown(s) && s.terminalSearch === s.activeTab ? s.activeTab : null,
  );
  const pane = id === null ? undefined : paneOf(id);
  return id !== null && pane ? createPortal(<SearchBar key={id} id={id} />, pane) : null;
}
