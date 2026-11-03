import { type RefObject, useEffect, useRef } from "react";
import { leaveView, type OpenFile, saveRenderedScroll, takePlace, useHive } from "../store";
import { isFor } from "./buffer";

/**
 * The same place in a Markdown file's text and in its rendered view (21.3). A place is a line
 * of the text plus how far into it (12.25 is a quarter into line 12). The rendered view's
 * top-level blocks carry their first line (`data-line`, `markLines`); a line maps to the block
 * holding it, proportionally inside it by line, and back.
 */

/** A rendered top-level block: its first line, and where it lies in the scroller's content. */
export type Block = { line: number; top: number; height: number };

/** A scroll that lands a pixel or so off a block's top (rounding) still counts as its top. */
const SLACK = 1;

/** The lines `blocks[i]` holds: up to the next block's, or to the end of the text's `lines`. */
function span(blocks: Block[], i: number, lines: number): number {
  const block = blocks[i] as Block;
  const end = blocks[i + 1]?.line ?? lines + 1;
  return Math.max(1, end - block.line);
}

/** The last block for which `before` holds (blocks are in order), or -1. */
function lastWhere(blocks: Block[], before: (b: Block) => boolean): number {
  let found = -1;
  for (let i = 0; i < blocks.length && before(blocks[i] as Block); i++) found = i;
  return found;
}

/** The scroll that puts `line` at the top: in the block holding it; 0 above the first block. */
export function scrollForLine(blocks: Block[], line: number, lines: number): number {
  const i = lastWhere(blocks, (b) => b.line <= line);
  if (i < 0) return 0;
  const block = blocks[i] as Block;
  const into = Math.min(1, (line - block.line) / span(blocks, i, lines));
  return block.top + into * block.height;
}

/**
 * The line at the top for scroll `top`: in the top visible block, the first one still showing
 * below `top` (a scroll in the margin above a block is that block's first line); the end of the
 * last block when scrolled past them all, 1 when there are none.
 */
export function lineForScroll(blocks: Block[], top: number, lines: number): number {
  if (blocks.length === 0) return 1;
  const shown = blocks.findIndex((b) => b.top + b.height > top + SLACK);
  const i = shown < 0 ? blocks.length - 1 : shown;
  const block = blocks[i] as Block;
  const into = block.height > 0 ? (top - block.top) / block.height : 0;
  return block.line + Math.min(1, Math.max(0, into)) * span(blocks, i, lines);
}

/** The top-level blocks rendered in `scroller`, measured in its content's coordinates. */
export function blocksOf(scroller: HTMLElement): Block[] {
  const origin = scroller.getBoundingClientRect().top - scroller.scrollTop;
  return [...scroller.querySelectorAll<HTMLElement>(".markdown > [data-line]")].map((el) => {
    const { top, height } = el.getBoundingClientRect();
    return { line: Number(el.dataset.line), top: top - origin, height };
  });
}

/** How many lines `text` has, as the editor counts them ("\n" only). */
export const lineCount = (text: string) => text.split("\n").length;

/**
 * Keeps the rendered view's place for `file` showing `text` (21.3): its scroll, taken at every
 * scroll event (a hidden or detached element reads 0) and kept in the file's tab, put back once
 * the Markdown has rendered (the browser clamps it to the height). Switched to from the text, it
 * starts at the text's top line instead; switched to the text, the text starts at its top
 * block's line. Returns the ref for the scrolling element.
 */
export function useRenderedPlace(file: OpenFile, text: string): RefObject<HTMLDivElement | null> {
  const scroller = useRef<HTMLDivElement>(null);
  const lines = useRef(0);
  lines.current = lineCount(text);
  const { worktree, path } = file;
  // A passive effect, not a layout one: the text view leaves its place in its passive cleanup,
  // which React runs before this one, and the text view takes this one's in its passive mount.
  useEffect(() => {
    const at = { worktree, path };
    const el = scroller.current as HTMLDivElement;
    const place = takePlace(at);
    el.scrollTop =
      place === undefined
        ? (useHive.getState().openFiles.find((t) => isFor(t, at))?.renderedScroll ?? 0)
        : scrollForLine(blocksOf(el), place, lines.current);
    const seen = () => ({
      scroll: el.scrollTop,
      line: lineForScroll(blocksOf(el), el.scrollTop, lines.current),
    });
    // Not scrolled since the switch, it gives the text back its own place, exactly.
    let kept = place === undefined ? seen() : { scroll: el.scrollTop, line: place };
    const onScroll = () => {
      // The scroll put back fires its own event: only a move changes the place.
      if (el.scrollTop !== kept.scroll) kept = seen();
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      saveRenderedScroll(at, kept.scroll);
      leaveView(at, false, kept.line);
    };
  }, [worktree, path]);
  return scroller;
}
