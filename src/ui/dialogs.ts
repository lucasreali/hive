/**
 * Where an open `.dialog` sits (23.9) is the stylesheet's, from its `--dialog-height`: this keeps
 * that property at the tallest the dialog has been since it opened, so a dialog only ever moves
 * up while open (a folder list filtered as one types never makes it bob). Its first height is
 * set before its first paint, without the glide (`data-placed`, set a frame later, turns it on).
 */
export function placeDialogs(
  root: Node,
  Sizes: typeof ResizeObserver = ResizeObserver,
  later: (run: () => void) => void = requestAnimationFrame,
): () => void {
  const tallest = new WeakMap<HTMLElement, number>();
  const sizes = new Sizes((entries) => {
    for (const { target } of entries) {
      const dialog = target as HTMLDialogElement;
      // A dialog React removed while open: nothing more to place.
      if (!dialog.isConnected || !dialog.open) {
        sizes.unobserve(dialog);
        continue;
      }
      const height = Math.max(tallest.get(dialog) ?? 0, dialog.offsetHeight);
      tallest.set(dialog, height);
      dialog.style.setProperty("--dialog-height", `${height}px`);
      if (!("placed" in dialog.dataset)) later(() => (dialog.dataset.placed = ""));
    }
  });
  const opened = new MutationObserver((records) => {
    for (const { target } of records) {
      if (!(target instanceof HTMLDialogElement) || !target.classList.contains("dialog")) continue;
      if (target.open) {
        sizes.observe(target);
        continue;
      }
      // Closed: placed afresh when it opens again.
      sizes.unobserve(target);
      tallest.delete(target);
      target.style.removeProperty("--dialog-height");
      delete target.dataset.placed;
    }
  });
  opened.observe(root, { subtree: true, attributes: true, attributeFilter: ["open"] });
  return () => {
    opened.disconnect();
    sizes.disconnect();
  };
}
