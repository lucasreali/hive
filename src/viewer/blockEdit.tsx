import { Text } from "@codemirror/state";
import {
  type MouseEvent,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { setEdit, setEditing, showNotice, useHive } from "../store";

// A Markdown preview's block edited as its source (25.4): clicking a top-level block (21.3's
// `data-line`, with its `data-line-end`) swaps that block alone for its lines in a small editor.

/** A block's lines in a text, from 1, and how its line endings are shown and written back. */
export type BlockSource = {
  first: number;
  last: number;
  /** Its lines with "\n" only: a text area would turn a "\r\n" into that when read. */
  source: string;
  /** Its lines end in "\r\n", so each one written back does. */
  crlf: boolean;
  /** Its last line ends in "\r" too; the file's last line, with no line ending, does not. */
  closed: boolean;
};

const ended = (line: string) => line.endsWith("\r");

/** A "\r" alone: a line ending to the Markdown parser, none to the buffer (`toText`). */
export const hasLoneCr = (text: string) => /\r(?!\n)/.test(text);

/** The source of the block on lines `first` to `last` of `doc`; null when `doc` has no such lines. */
export function blockSource(doc: Text, first: number, last: number): BlockSource | null {
  if (!(first >= 1 && first <= doc.lines && last >= first)) return null;
  const end = Math.min(last, doc.lines);
  const raw = doc.sliceString(doc.line(first).from, doc.line(end).to).split("\n");
  const closed = ended(raw[raw.length - 1] as string);
  // A single line says it itself, unless it is the file's last, with no line ending: then the
  // line before it does.
  const before = end === doc.lines && first > 1 && ended(doc.line(first - 1).text);
  const crlf = raw.slice(0, -1).every(ended) && (raw.length > 1 || closed || before);
  const source = (crlf ? raw.map((line) => line.replace(/\r$/, "")) : raw).join("\n");
  return { first, last: end, source, crlf, closed };
}

/** `doc` with the block's lines replaced by `draft`; every other byte stays. */
export function withBlock(doc: Text, block: BlockSource, draft: string): Text {
  const lines = draft.split("\n");
  const written = block.crlf
    ? lines.map((line, i) => (i < lines.length - 1 || block.closed ? `${line}\r` : line))
    : lines;
  return doc.replace(doc.line(block.first).from, doc.line(block.last).to, Text.of(written));
}

/** What a click on a rendered block means nothing to: its links, buttons and the like. */
const OWN_CLICK = "a, button, summary, input, select, textarea";

/** The block a click on the preview opens: none on a control, or ending a text selection. */
export function clickedBlock(target: Element): HTMLElement | null {
  if (target.closest(OWN_CLICK) || !window.getSelection()?.isCollapsed) return null;
  return target.closest<HTMLElement>("[data-line]");
}

type Opened = BlockSource & {
  /** The text rendered until it closes, so the rest of the preview stays as it is. */
  shown: string;
  base: Text;
  block: HTMLElement;
  /** Where the editor goes, next to the hidden block. */
  host: HTMLElement;
};

/** `el` as tall as its text. */
function fit(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight + 2}px`;
}

/** The small editor, as tall as its text: the browser's own undo, Escape closes it. */
function BlockEditor(props: {
  source: string;
  label: string;
  onChange: (draft: string) => void;
  onClose: () => void;
}) {
  const box = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = box.current as HTMLTextAreaElement;
    fit(el);
    el.focus();
  }, []);
  return (
    <textarea
      ref={box}
      className="md-block-editor"
      aria-label={props.label}
      defaultValue={props.source}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      onInput={(e) => {
        fit(e.currentTarget);
        props.onChange(e.currentTarget.value);
      }}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        props.onClose();
      }}
    />
  );
}

/**
 * Editing the blocks of the Markdown preview of `text` (`text` is the open file's buffer or its
 * answer). Each keystroke reaches the buffer at once, so the tab's dirty mark and Save work as
 * for the text, while the preview keeps rendering the text it had (`shown`): only closing the
 * block renders the change. Escape or a click outside closes it; a new text from elsewhere (a
 * reload from disk) too, since its lines are no longer the block's.
 */
export function useBlockEdit(text: string): {
  shown: string;
  /** Whether a block was edited in this view since it opened. */
  used: () => boolean;
  onClick: (event: MouseEvent) => void;
  editor: ReactNode;
} {
  const [open, setOpen] = useState<Opened | null>(null);
  // What the buffer holds as far as this editor knows: anything else is a change from elsewhere.
  const written = useRef<Text | null>(null);
  const used = useRef(false);
  // The click that closed a block opens no other: only the next one does.
  const closedBy = useRef<Event | null>(null);
  const doc = useHive((s) => s.edit?.doc ?? null);

  useEffect(() => {
    if (!open) return;
    const { block, host } = open;
    return () => {
      host.remove();
      block.style.display = "";
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (open.host.contains(event.target as Node)) return;
      closedBy.current = event;
      setOpen(null);
    };
    document.addEventListener("click", outside, true);
    return () => document.removeEventListener("click", outside, true);
  }, [open]);

  if (open && doc !== written.current) setOpen(null);

  const onClick = (event: MouseEvent) => {
    if (open || closedBy.current === event.nativeEvent) return;
    const block = clickedBlock(event.target as Element);
    if (!block) return;
    // The rendered blocks' line numbers would not be the buffer's.
    if (hasLoneCr(text)) {
      showNotice("error", "This file has a line ending in \\r alone: edit it in the text");
      return;
    }
    if (!useHive.getState().edit) setEditing(true);
    const base = useHive.getState().edit?.doc;
    if (!base) return;
    const lines = blockSource(base, Number(block.dataset.line), Number(block.dataset.lineEnd));
    if (!lines) return;
    const host = document.createElement("div");
    block.after(host);
    block.style.display = "none";
    written.current = base;
    setOpen({ ...lines, shown: text, base, block, host });
  };

  const change = (draft: string) => {
    const { edit } = useHive.getState();
    // Not the buffer this editor wrote: gone, or another text the next render closes it for.
    if (!open || edit?.doc !== written.current) return;
    const next = withBlock(open.base, open, draft);
    written.current = next;
    used.current = true;
    setEdit({ ...edit, doc: next });
  };

  return {
    shown: open ? open.shown : text,
    used: () => used.current,
    onClick,
    editor:
      open &&
      createPortal(
        <BlockEditor
          source={open.source}
          label={`Markdown of lines ${open.first} to ${open.last}`}
          onChange={change}
          onClose={() => setOpen(null)}
        />,
        open.host,
      ),
  };
}
