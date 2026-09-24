import type { KeyboardEvent, PointerEvent } from "react";
import {
  LIMITS,
  PANEL_CLOSE_AT,
  type Side,
  setRightPanel,
  setWidth,
  useHive,
  widthKey,
} from "../store";

/** How far an arrow key moves the edge. */
const STEP = 16;

/**
 * The draggable edge of the left sidebar (its right edge) or of the right panel (its left
 * edge). Dragging sizes it within its limits; dragging the right panel much narrower than its
 * minimum closes it. ←/→ move the edge from the keyboard.
 */
export function ResizeHandle({ side }: { side: Side }) {
  const width = useHive((s) => s[widthKey(side)]);
  const label = side === "sidebar" ? "Resize the sidebar" : "Resize the side panel";
  const drag = (event: PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const handle = event.currentTarget;
    handle.setPointerCapture?.(event.pointerId);
    document.body.dataset.resizing = "true";
    const move = (e: globalThis.PointerEvent) => {
      const wanted = side === "sidebar" ? e.clientX : window.innerWidth - e.clientX;
      if (side === "panel" && wanted < PANEL_CLOSE_AT) {
        stop();
        return setRightPanel(null);
      }
      setWidth(side, wanted);
    };
    const stop = () => {
      delete document.body.dataset.resizing;
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", stop);
      handle.removeEventListener("pointercancel", stop);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", stop);
    handle.addEventListener("pointercancel", stop);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowLeft: -STEP, ArrowRight: STEP }[event.key];
    if (!step) return;
    event.preventDefault();
    // The right panel's edge is its left one: moving it right makes it narrower.
    setWidth(side, width + (side === "sidebar" ? step : -step));
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable, movable separator has no element.
    <div
      className="resize-handle"
      data-side={side}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={LIMITS[side].min}
      aria-valuemax={LIMITS[side].max}
      tabIndex={0}
      onPointerDown={drag}
      onKeyDown={onKeyDown}
    />
  );
}
