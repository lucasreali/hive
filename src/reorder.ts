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

/**
 * Native HTML5 drag and drop to reorder one list (8.2 agents, 8.21 tabs). `group` names the list:
 * an item dragged from another group is refused. `move(id, target, after)` gets the drop. Returns
 * the props for each item, including `data-drop="before" | "after"` where the drop line shows.
 * `horizontal` compares the pointer with the item's middle along x instead of y.
 */
export function useReorder(
  group: string,
  move: (id: string, target: string, after: boolean) => void,
  horizontal = false,
) {
  const [line, setLine] = useState<Line | null>(null);
  const side = (e: DragEvent<HTMLElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    return horizontal ? e.clientX > box.left + box.width / 2 : e.clientY > box.top + box.height / 2;
  };
  const takes = (id: string) => dragged?.group === group && dragged.id !== id;
  return (id: string) => ({
    draggable: true,
    "data-drop": line?.target === id ? (line.after ? "after" : "before") : undefined,
    onDragStart: (e: DragEvent<HTMLElement>) => {
      dragged = { group, id };
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("application/x-hive-reorder", id);
    },
    onDragOver: (e: DragEvent<HTMLElement>) => {
      if (!takes(id)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const after = side(e);
      if (line?.target !== id || line.after !== after) setLine({ target: id, after });
    },
    onDragLeave: (e: DragEvent<HTMLElement>) => {
      // Moving onto the item's own children is not leaving it.
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setLine(null);
    },
    onDrop: (e: DragEvent<HTMLElement>) => {
      setLine(null);
      if (!dragged || !takes(id)) return;
      e.preventDefault();
      move(dragged.id, id, side(e));
    },
    onDragEnd: () => {
      dragged = null;
      setLine(null);
    },
  });
}
