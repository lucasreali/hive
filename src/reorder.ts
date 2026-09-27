import { type DragEvent, useState } from "react";

/** `list` with `id` moved next to `target` (after it when `after`); unchanged if either is missing. */
export function moveNextTo<T>(list: T[], id: T, target: T, after: boolean): T[] {
  if (id === target || !list.includes(id) || !list.includes(target)) return list;
  const rest = list.filter((x) => x !== id);
  const at = rest.indexOf(target) + (after ? 1 : 0);
  return [...rest.slice(0, at), id, ...rest.slice(at)];
}

/** The item being dragged and its list: only that list takes it (one drag at a time). */
let dragged: { group: string; id: string } | null = null;

/** Where the drop line shows: on which item, and on which side of it. */
type Line = { target: string; after: boolean };

/** Whether the pointer of a drag event is on the far side (after) of its target. */
type Side = (e: DragEvent<HTMLElement>) => boolean;

/**
 * Native HTML5 drag and drop to reorder one list (8.2 agents, 8.21 tabs). `group` names the list:
 * an item dragged from another group is refused. `move(id, target, after)` gets the drop. Returns
 * the props for each item, including `data-drop="before" | "after"` where the drop line shows and
 * `data-dragging` on the item picked up. `horizontal` compares the pointer with the item's middle
 * along x instead of y. `.end(last)` gives the list container's props: a drop on its empty part,
 * past every item, lands after `last` (9.33).
 */
export function useReorder(
  group: string,
  move: (id: string, target: string, after: boolean) => void,
  horizontal = false,
) {
  const [line, setLine] = useState<Line | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const side: Side = (e) => {
    const box = e.currentTarget.getBoundingClientRect();
    return horizontal ? e.clientX > box.left + box.width / 2 : e.clientY > box.top + box.height / 2;
  };
  const takes = (id: string) => dragged?.group === group && dragged.id !== id;
  /** The drop handlers of `id`; `after` tells on which side of it the pointer is. */
  const target = (id: string, after: Side) => ({
    onDragOver: (e: DragEvent<HTMLElement>) => {
      if (!takes(id)) {
        // Over the dragged item itself, or one that refuses it: no line.
        if (line) setLine(null);
        return;
      }
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const a = after(e);
      if (line?.target !== id || line.after !== a) setLine({ target: id, after: a });
    },
    onDragLeave: (e: DragEvent<HTMLElement>) => {
      // Onto its own children, or onto another item (whose dragover moves or clears the line),
      // is not leaving: the line does not blink off in between.
      const to = e.relatedTarget as Element | null;
      if (!e.currentTarget.contains(to) && !to?.closest("[draggable='true']")) setLine(null);
    },
    onDrop: (e: DragEvent<HTMLElement>) => {
      setLine(null);
      if (!dragged || !takes(id)) return;
      e.preventDefault();
      move(dragged.id, id, after(e));
    },
  });
  const item = (id: string) => ({
    draggable: true,
    "data-drop": line?.target === id ? (line.after ? "after" : "before") : undefined,
    "data-dragging": source === id || undefined,
    onDragStart: (e: DragEvent<HTMLElement>) => {
      dragged = { group, id };
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("application/x-hive-reorder", id);
      // Dimmed once the browser has taken the drag image, so the image is not.
      setTimeout(() => dragged?.id === id && setSource(id));
    },
    ...target(id, side),
    onDragEnd: () => {
      dragged = null;
      setLine(null);
      setSource(null);
    },
  });
  const end = (last: string | undefined) => {
    const drop = target(last ?? "", () => true);
    // What happens over an item is the item's.
    const empty = (e: DragEvent<HTMLElement>) =>
      last !== undefined && !(e.target as Element).closest("[draggable='true']");
    return {
      onDragOver: (e: DragEvent<HTMLElement>) => empty(e) && drop.onDragOver(e),
      onDragLeave: drop.onDragLeave,
      onDrop: (e: DragEvent<HTMLElement>) => empty(e) && drop.onDrop(e),
    };
  };
  return Object.assign(item, { end });
}
