import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { notice } from "../../test/notice";
import { initialState, useHive } from "../store";
import { VERCEL_COMMENT } from "../transport/mockPulls";

// Counts the parses (renders of react-markdown) to check the memo; it still renders.
let parses = 0;
const real = ReactMarkdown;
mock.module("react-markdown", () => ({
  default: (props: Parameters<typeof real>[0]) => {
    parses++;
    return real(props);
  },
}));
const { Markdown, openLink, safeUrl } = await import("./Markdown");

afterEach(() => {
  cleanup();
  clearMocks();
  useHive.setState(initialState, true);
});

const md = (text: string) => render(<Markdown text={text} />).container;

test("headings, emphasis, inline code and lists", () => {
  const view = md(
    "## Context Usage\n\n**Model:** *opus* ~~old~~ `x = 1`\n\n- one\n- two\n\n1. first",
  );
  expect(view.querySelector("h2")?.textContent).toBe("Context Usage");
  expect(view.querySelector("strong")?.textContent).toBe("Model:");
  expect(view.querySelector("em")?.textContent).toBe("opus");
  expect(view.querySelector("del")?.textContent).toBe("old");
  expect(view.querySelector("p code")?.textContent).toBe("x = 1");
  expect([...view.querySelectorAll("ul li")].map((li) => li.textContent)).toEqual(["one", "two"]);
  expect(view.querySelector("ol li")?.textContent).toBe("first");
});

test("a fenced code block is a pre with its language", () => {
  const view = md("```sh\nls -la\n```");
  const code = view.querySelector("pre > code");
  expect(code?.textContent).toBe("ls -la\n");
  expect(code?.className).toBe("language-sh");
});

test("a GFM table has its header, cells and alignment, inside a scroller", () => {
  const view = md("| Name | Tokens |\n| :--- | ---: |\n| System | 3,100 |");
  const table = view.querySelector(".md-table > table");
  expect([...(table?.querySelectorAll("th") ?? [])].map((th) => th.textContent)).toEqual([
    "Name",
    "Tokens",
  ]);
  const cells = table?.querySelectorAll("td") ?? [];
  expect([...cells].map((td) => td.textContent)).toEqual(["System", "3,100"]);
  expect((cells[0] as HTMLElement).style.textAlign).toBe("left");
  expect((cells[1] as HTMLElement).style.textAlign).toBe("right");
});

test("a cell of one word never breaks; a cell of several words, or none, wraps (20.1)", () => {
  const view = md("| Approved | Reason | |\n|---|---|---|\n| 2026-09-23 | Pure wiring. | |");
  const cls = (sel: string) => [...view.querySelectorAll(sel)].map((c) => c.className);
  expect(cls("th")).toEqual(["md-word", "md-word", ""]);
  expect(cls("td")).toEqual(["md-word", "", ""]);
});

test("Vercel's comment: its HTML links are links, its date is text, no tag shows (D20.2)", () => {
  const view = md(VERCEL_COMMENT);
  expect(
    [...view.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]),
  ).toEqual([
    ["Vercel for GitHub", "https://vercel.link/github-learn-more"],
    ["dashboards-one", "https://vercel.com/user/dashboards-one"],
    ["Ready", "https://vercel.com/user/dashboards-one/Ab12"],
    ["Preview", "https://dashboards-one-git-feature-user.vercel.app"],
    [
      "Request Review",
      "https://vercel.com/vercel-agent/request-review?owner=user&repo=dashboards-one&pr=2",
    ],
  ]);
  // The avatar link (an image with an empty alt) is dropped; no image or source loads.
  expect(view.querySelector("img, source, sup:empty")).toBeNull();
  expect(view.querySelector("a picture .md-img")?.textContent).toBe("Request Review");
  const cells = [...view.querySelectorAll("td")].map((td) => td.textContent?.trim());
  expect(cells).toEqual(["dashboards-one", "Ready Ready", "Preview", "Oct 26, 2026 6:46pm UTC"]);
  expect(view.querySelector("relative-time")).toBeNull();
  expect(view.textContent).not.toContain("<");
  expect(view.textContent).not.toContain("vc]:");
});

test("raw HTML: nothing executable survives, other tags keep only their text (D20.2)", () => {
  const view = md(
    '<script>alert(1)</script>\n\nhi <b onclick="x()" style="color:red">bold</b> <img src=x onerror=y>' +
      ' <iframe src="https://evil.dev"></iframe> <a href="javascript:alert(1)">js</a>' +
      ' <a href="https://a.dev" onclick="x()" style="color:red" target="_blank">ok</a>' +
      " <sup>up</sup><sub>down</sub>a<br>b\n\n<style>p{}</style>\n\n" +
      "<details open><summary>More</summary>\n\nhidden\n\n</details>",
  );
  expect(view.querySelector("script, style, iframe, b, img")).toBeNull();
  expect(view.textContent).not.toContain("alert");
  expect(view.textContent).not.toContain("p{}");
  expect(view.textContent).toContain("hi bold");
  // A refused href is text; an allowed link keeps only its href.
  const links = [...view.querySelectorAll("a")];
  expect(links.map((a) => a.getAttribute("href"))).toEqual(["https://a.dev"]);
  expect(links[0]?.getAttributeNames().sort()).toEqual(["href", "title"]);
  expect(view.textContent).toContain("js");
  for (const el of view.querySelectorAll("*")) {
    expect(el.getAttributeNames().filter((n) => n === "style" || n.startsWith("on"))).toEqual([]);
  }
  expect(view.querySelector("sup")?.textContent).toBe("up");
  expect(view.querySelector("sub")?.textContent).toBe("down");
  expect(view.querySelector("br")).not.toBeNull();
  expect(view.querySelector("details")?.hasAttribute("open")).toBe(false);
  expect(view.querySelector("details > summary")?.textContent).toBe("More");
});

test("raw tables nested deep render without overflowing the stack; align takes GFM's values only", () => {
  const view = md(`${"<table><tr><td>".repeat(600)}deep x`);
  expect(view.querySelectorAll("td")).toHaveLength(600);
  expect(view.querySelector("td")?.textContent).toBe("deep x");
  const cells = md(
    '<table><tr><td align="center;background:url(http://e/x)">a</td><td align="right">b</td></tr></table>',
  ).querySelectorAll("td");
  expect([...cells].map((td) => (td as HTMLElement).style.cssText)).toEqual([
    "",
    "text-align: right;",
  ]);
});

test("a task checkbox stays a disabled checkbox; a raw input is never editable", () => {
  const view = md('- [x] done\n- [ ] todo\n\n<input type="text" value="typed">');
  const boxes = [...view.querySelectorAll("input")] as HTMLInputElement[];
  expect(boxes.map((b) => [b.type, b.disabled, b.checked])).toEqual([
    ["checkbox", true, true],
    ["checkbox", true, false],
    ["checkbox", true, false],
  ]);
  expect(view.querySelector("ul")?.className).toBe("contains-task-list");
  expect(view.querySelector("li")?.className).toBe("task-list-item");
});

test("images are never loaded: the alt text stands in", () => {
  const view = md("![a chart](https://example.com/x.png) ![](https://example.com/y.png)");
  expect(view.querySelector("img")).toBeNull();
  expect(view.querySelector(".md-img")?.textContent).toBe("a chart");
  expect(view.querySelectorAll(".md-img")).toHaveLength(1);
});

test("only http(s) and mailto links are links", () => {
  const view = md(
    "[web](https://a.dev) [plain](http://a.dev) [mail](mailto:x@a.dev) " +
      "[bad](javascript:alert(1)) [ent](java&#115;cript:alert(1)) [file](file:///etc/passwd) [rel](../x)",
  );
  expect([...view.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
    "https://a.dev",
    "http://a.dev",
    "mailto:x@a.dev",
  ]);
  expect(view.textContent).toContain("bad ent file rel");
  expect(safeUrl(" JavaScript:x")).toBe("");
  expect(safeUrl("HTTPS://a.dev")).toBe("HTTPS://a.dev");
  expect(safeUrl("mailto:x@a.dev?body=hi&attach=/home/u/.ssh/id_rsa")).toBe("mailto:x@a.dev");
});

test("a click opens the link outside the app and the webview does not navigate", async () => {
  const calls: [string, unknown][] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
  });
  const link = md("[web](https://a.dev)").querySelector("a") as HTMLAnchorElement;
  // Outside Tauri (the browser mock): nothing opens, the status bar says so.
  const click = new MouseEvent("click", { bubbles: true, cancelable: true });
  link.dispatchEvent(click);
  expect(click.defaultPrevented).toBe(true);
  expect(notice()).toBe("Only the Hive app opens links: https://a.dev");
  expect(calls).toEqual([]);
  // A middle-click opens outside too; another button does nothing; neither navigates.
  useHive.setState({ notices: [] });
  const aux = (button: number) => {
    const event = new MouseEvent("auxclick", { bubbles: true, cancelable: true, button });
    link.dispatchEvent(event);
    return event.defaultPrevented;
  };
  expect(aux(2)).toBe(true);
  expect(notice()).toBeNull();
  expect(aux(1)).toBe(true);
  expect(notice()).toBe("Only the Hive app opens links: https://a.dev");

  await openLink("https://a.dev", true);
  expect(calls).toEqual([["plugin:opener|open_url", { url: "https://a.dev" }]]);
  mockIPC(() => {
    throw new Error("no browser");
  });
  await openLink("https://a.dev", true);
  expect(notice()).toContain("no browser");
});

test("the same text is not parsed again; a new text is", () => {
  const { rerender } = render(<Markdown text="one" />);
  const before = parses;
  rerender(<Markdown text="one" />);
  expect(parses).toBe(before);
  rerender(<Markdown text="one two" />);
  expect(parses).toBe(before + 1);
});

// 14.6: a file's preview is a document, its fenced code coloured by the editor's parsers.
const doc = (text: string) => render(<Markdown text={text} document />).container;

test("a file's preview colours a fenced block by its language, without the closing newline", async () => {
  const view = doc('```ts\nexport function f(x: number) {\n  return "a" + 1; // b\n}\n```');
  expect(view.querySelector(".markdown")?.classList).toContain("markdown-doc");
  const code = view.querySelector("pre > code") as HTMLElement;
  expect(code.className).toBe("language-ts");
  // Plain until the language loads.
  expect(code.querySelector("span")).toBeNull();
  // The parser is a lazy chunk: the first load in a test run can take a while.
  await waitFor(() => expect(code.querySelector(".tok-keyword")?.textContent).toBe("export"), {
    timeout: 4000,
  });
  const cls = (c: string) => [...code.querySelectorAll(`.tok-${c}`)].map((s) => s.textContent);
  expect(cls("keyword")).toEqual(["export", "function", "return"]);
  expect(cls("type")).toEqual(["number", "1"]);
  expect(cls("string")).toEqual(['"a"']);
  expect(cls("comment")).toEqual(["// b"]);
  expect(cls("operator")).toEqual(["+"]);
  expect(code.textContent).toBe('export function f(x: number) {\n  return "a" + 1; // b\n}');
});

test("a file's preview leaves inline code, an unknown language and no language plain", () => {
  const lookup = spyOn(LanguageDescription, "matchLanguageName");
  const view = doc("`let x`\n\n```nolang\nlet y\n```\n\n```\nlet z\n```");
  // Only the fenced block with a language is looked up, and nothing is found: nothing loads.
  expect(lookup.mock.calls).toEqual([[languages, "nolang", true]]);
  expect(lookup.mock.results.map((r) => r.value)).toEqual([null]);
  lookup.mockRestore();
  expect(view.querySelector("p code")?.textContent).toBe("let x");
  const blocks = [...view.querySelectorAll("pre > code")];
  expect(blocks.map((c) => [c.className, c.textContent])).toEqual([
    ["language-nolang", "let y"],
    ["", "let z"],
  ]);
  expect(view.querySelector("span")).toBeNull();
});

test("a block whose parser fails to load stays plain", async () => {
  const ts = LanguageDescription.matchLanguageName(languages, "ts", true) as LanguageDescription;
  const failed = Promise.reject(new Error("chunk failed"));
  const load = spyOn(ts, "load").mockReturnValue(failed);
  const view = doc("```ts\nexport const a = 1;\n```");
  expect(load).toHaveBeenCalledTimes(1);
  // The rejection is caught (an unhandled one fails the run); the block keeps its text.
  await act(async () => {
    await failed.catch(() => {});
  });
  load.mockRestore();
  expect(view.querySelector("pre > code")?.textContent).toBe("export const a = 1;");
  expect(view.querySelector("pre span")).toBeNull();
});

test("a pull request's text is not a document: its code stays plain", () => {
  const lookup = spyOn(LanguageDescription, "matchLanguageName");
  const view = md("```ts\nexport const a = 1;\n```");
  // No language is looked up, so nothing can colour it later.
  expect(lookup).not.toHaveBeenCalled();
  lookup.mockRestore();
  expect(view.querySelector(".markdown")?.classList).not.toContain("markdown-doc");
  expect(view.querySelector("pre > code")?.textContent).toBe("export const a = 1;\n");
  expect(view.querySelector("pre span")).toBeNull();
});
