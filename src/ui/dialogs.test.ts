import { afterEach, expect, test } from "bun:test";
import { placeDialogs } from "./dialogs";

/** A ResizeObserver whose sizes the test reports itself, with what it observes. */
class Sizes {
  static last: Sizes;
  observed = new Set<Element>();
  constructor(readonly report: (entries: { target: Element }[]) => void) {
    Sizes.last = this;
  }
  observe(el: Element) {
    this.observed.add(el);
  }
  unobserve(el: Element) {
    this.observed.delete(el);
  }
  disconnect() {
    this.observed.clear();
  }
}

const frames: (() => void)[] = [];
const later = (run: () => void) => frames.push(run);
/** The changes the observers wait for (a mutation record), then each frame asked for. */
const settle = async () => {
  await new Promise((done) => setTimeout(done, 0));
  for (const run of frames.splice(0)) run();
};

let stop = () => {};
afterEach(() => {
  stop();
  document.body.innerHTML = "";
  frames.length = 0;
});

/** A `.dialog` (or another `<dialog>` with `className`) whose height is `height()`. */
function dialog(height: () => number, className = "dialog") {
  const el = document.createElement("dialog");
  el.className = className;
  Object.defineProperty(el, "offsetHeight", { get: height });
  document.body.append(el);
  return el;
}

const start = () => {
  stop = placeDialogs(document.body, Sizes as unknown as typeof ResizeObserver, later);
};
const height = (el: HTMLElement) => el.style.getPropertyValue("--dialog-height");
const report = (el: Element) => Sizes.last.report([{ target: el }]);

test("an opened dialog gets its height, then its glide a frame later; a short one too", async () => {
  start();
  let h = 200;
  const el = dialog(() => h);
  el.showModal();
  await new Promise((done) => setTimeout(done, 0));
  expect(Sizes.last.observed.has(el)).toBe(true);
  report(el);
  expect(height(el)).toBe("200px");
  // Not yet: its first place is taken without the transition.
  expect("placed" in el.dataset).toBe(false);
  await settle();
  expect(el.dataset.placed).toBe("");
  // Taller: it follows; shorter again (a list filtered as one types): it keeps the tallest.
  h = 640;
  report(el);
  expect(height(el)).toBe("640px");
  h = 300;
  report(el);
  expect(height(el)).toBe("640px");
  expect(frames).toHaveLength(0);
});

test("closed, a dialog forgets its height and is placed afresh when it opens again", async () => {
  start();
  let h = 640;
  const el = dialog(() => h);
  el.showModal();
  await settle();
  report(el);
  await settle();
  el.close();
  await settle();
  expect(Sizes.last.observed.has(el)).toBe(false);
  expect(height(el)).toBe("");
  expect("placed" in el.dataset).toBe(false);
  h = 200;
  el.showModal();
  await settle();
  report(el);
  expect(height(el)).toBe("200px");
});

test("a dialog removed while open, or reported once closed, is no longer observed", async () => {
  start();
  const el = dialog(() => 300);
  el.showModal();
  await settle();
  el.remove();
  report(el);
  expect(Sizes.last.observed.has(el)).toBe(false);
  expect(height(el)).toBe("");
  const other = dialog(() => 300);
  other.showModal();
  await settle();
  // Its size came in after it closed, before the close was seen.
  other.removeAttribute("open");
  report(other);
  expect(Sizes.last.observed.has(other)).toBe(false);
  expect(height(other)).toBe("");
});

test("only `.dialog` dialogs are placed, and stopping lets every dialog go", async () => {
  start();
  const plain = dialog(() => 300, "");
  plain.showModal();
  const details = document.createElement("details");
  details.classList.add("dialog");
  document.body.append(details);
  details.open = true;
  const el = dialog(() => 300);
  el.showModal();
  await settle();
  expect([...Sizes.last.observed]).toEqual([el]);
  stop();
  expect(Sizes.last.observed.size).toBe(0);
});
