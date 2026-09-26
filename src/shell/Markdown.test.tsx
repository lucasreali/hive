import { afterEach, expect, mock, test } from "bun:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { initialState, useHive } from "../store";

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

test("raw HTML shows as text, never as elements", () => {
  const view = md(
    '<script>alert(1)</script>\n\nhi <b onclick="x()">bold</b> <img src=x onerror=y>',
  );
  expect(view.querySelector("script, b, img")).toBeNull();
  expect(view.textContent).toContain("<script>alert(1)</script>");
  expect(view.textContent).toContain('<b onclick="x()">bold</b>');
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
  expect(useHive.getState().notice).toBe("Only the Hive app opens links: https://a.dev");
  expect(calls).toEqual([]);

  await openLink("https://a.dev", true);
  expect(calls).toEqual([["plugin:opener|open_url", { url: "https://a.dev" }]]);
  mockIPC(() => {
    throw new Error("no browser");
  });
  await openLink("https://a.dev", true);
  expect(useHive.getState().notice).toContain("no browser");
});

test("the same text is not parsed again; a new text is", () => {
  const { rerender } = render(<Markdown text="one" />);
  const before = parses;
  rerender(<Markdown text="one" />);
  expect(parses).toBe(before);
  rerender(<Markdown text="one two" />);
  expect(parses).toBe(before + 1);
});
