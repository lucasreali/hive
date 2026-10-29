import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { highlightCode, tags as t, tagHighlighter } from "@lezer/highlight";
import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Element, ElementContent, Root } from "hast";
import { memo, type ReactNode, useEffect, useState } from "react";
import ReactMarkdown, { type Components, type Options } from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { type Options as Schema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { showNotice } from "../store";

/**
 * Only these links open (8.6): anything else (`javascript:`, `file:`, relative) is plain text.
 * A `mailto:` keeps its address only: no prefilled body or `attach=` for the mail app.
 */
export function safeUrl(url: string): string {
  const trimmed = url.trim();
  if (/^mailto:/i.test(trimmed)) return trimmed.split("?")[0] as string;
  return /^https?:/i.test(trimmed) ? trimmed : "";
}

/**
 * A link opens outside the app, in the default browser or mail app (the opener plugin); the
 * webview never navigates. Outside Tauri (browser, mock transport) nothing opens and the status
 * bar says so.
 */
export async function openLink(url: string, tauri = isTauri()): Promise<void> {
  if (!tauri) return showNotice("error", `Only the Hive app opens links: ${url}`);
  try {
    await openUrl(url);
  } catch (error) {
    showNotice("error", String(error));
  }
}

/** The editor's One Dark groups (`oneDarkHighlightStyle`) as classes, coloured per theme. */
const HIGHLIGHTER = tagHighlighter([
  { tag: t.keyword, class: "tok-keyword" },
  { tag: [t.name, t.deleted, t.character, t.propertyName, t.macroName], class: "tok-name" },
  { tag: [t.function(t.variableName), t.labelName], class: "tok-function" },
  { tag: [t.color, t.constant(t.name), t.standard(t.name)], class: "tok-constant" },
  { tag: [t.definition(t.name), t.separator], class: "tok-plain" },
  {
    tag: [t.typeName, t.className, t.number, t.changed, t.annotation, t.modifier, t.self],
    class: "tok-type",
  },
  { tag: [t.operator, t.operatorKeyword, t.url, t.escape, t.regexp], class: "tok-operator" },
  { tag: [t.meta, t.comment], class: "tok-comment" },
  { tag: [t.atom, t.bool, t.special(t.variableName)], class: "tok-constant" },
  { tag: [t.processingInstruction, t.string, t.inserted], class: "tok-string" },
]);

/**
 * A fenced block's code coloured by the editor's own parsers (`tok-*` classes, coloured by the
 * theme's tokens in styles.css); plain until its language loads, and plain for an unknown one
 * or one whose parser fails to load.
 */
function Code({ lang, text }: { lang: string; text: string }) {
  const [nodes, setNodes] = useState<ReactNode[] | null>(null);
  useEffect(() => {
    let live = true;
    void LanguageDescription.matchLanguageName(languages, lang, true)
      ?.load()
      .then(({ language }) => {
        const out: ReactNode[] = [];
        highlightCode(
          text,
          language.parser.parse(text),
          HIGHLIGHTER,
          (part, cls) =>
            out.push(
              cls ? (
                <span key={out.length} className={cls}>
                  {part}
                </span>
              ) : (
                part
              ),
            ),
          () => out.push("\n"),
        );
        if (live) setNodes(out);
      })
      // A parser that fails to load leaves the block plain: the code still reads.
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [lang, text]);
  return <code className={`language-${lang}`}>{nodes ?? text}</code>;
}

/**
 * What survives of the HTML in a text (D20.2): the elements Markdown itself renders, and of raw
 * HTML only `a`, `img` / `picture` (never loaded: the alt stands in), `sup`, `sub`, `br`,
 * `details` / `summary`. Any other tag (`relative-time`, `iframe`…) keeps only its text, a
 * `script` or `style` not even that. No attribute but `href` and `alt`, and the classes, task
 * checkboxes and cell alignment Markdown sets itself: no `style`, no event handler, no `src`.
 */
const SCHEMA: Schema = {
  tagNames: [
    ...["p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "code"],
    ...["em", "strong", "del", "hr", "table", "thead", "tbody", "tr", "th", "td", "input"],
    ...["a", "img", "picture", "sup", "sub", "br", "details", "summary"],
  ],
  attributes: {
    a: ["href"],
    img: ["alt"],
    code: [["className", /^language-./]],
    ul: [["className", "contains-task-list"]],
    ol: [["className", "contains-task-list"]],
    li: [["className", "task-list-item"]],
    input: [["type", "checkbox"], ["disabled", true], "checked"],
    // Only the values GFM sets: `align` becomes the cell's `text-align`.
    th: [["align", "left", "center", "right"]],
    td: [["align", "left", "center", "right"]],
  },
  required: { input: { type: "checkbox", disabled: true } },
  protocols: { href: ["http", "https", "mailto"] },
  strip: ["script", "style"],
};

/**
 * The text a reader sees of a node: its text, and an image's alt (the image never loads).
 * A loop, not recursion: hostile text can nest raw tables hundreds deep (a stack overflow).
 */
function shown(node: ElementContent): string {
  let text = "";
  const stack = [node];
  for (let next = stack.pop(); next; next = stack.pop()) {
    if (next.type === "text") text += next.value;
    else if (next.type === "element" && next.tagName === "img")
      text += String(next.properties.alt ?? "");
    else if (next.type === "element")
      for (const child of [...next.children].reverse()) stack.push(child);
  }
  return text;
}

/** A link left with nothing to read (an avatar with an empty alt) is dropped (D20.2). */
function pruneEmptyLinks(tree: Root) {
  const stack: (Root | Element)[] = [tree];
  for (let node = stack.pop(); node; node = stack.pop()) {
    node.children = node.children.filter(
      (child) => !(child.type === "element" && child.tagName === "a" && !shown(child).trim()),
    ) as typeof node.children;
    for (const child of node.children) if (child.type === "element") stack.push(child);
  }
}

const PLUGINS = [remarkGfm];
const REHYPE: Options["rehypePlugins"] = [
  rehypeRaw,
  [rehypeSanitize, SCHEMA],
  () => pruneEmptyLinks,
];
const DOC_COMPONENTS: Components = {
  // A block's text ends with its newline, which would show as an empty last line.
  code: ({ className, children }) => {
    const lang = /language-(\S+)/.exec(className ?? "")?.[1];
    const text = typeof children === "string" ? children.replace(/\n$/, "") : children;
    return lang && typeof text === "string" ? (
      <Code lang={lang} text={text} />
    ) : (
      <code className={className}>{text}</code>
    );
  },
};
const COMPONENTS: Components = {
  // A refused link (`safeUrl` gave "") is its text only.
  a: ({ href, children }) =>
    href ? (
      <a
        href={href}
        title={href}
        onClick={(event) => {
          event.preventDefault();
          void openLink(href);
        }}
        // A middle-click (auxclick, not click) would open a popup webview on Windows.
        onAuxClick={(event) => {
          event.preventDefault();
          if (event.button === 1) void openLink(href);
        }}
      >
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  // Never load an image (untrusted text, no network): its alt text stands in.
  img: ({ alt }) => (alt ? <span className="md-img">{alt}</span> : null),
  // A table is never wider than the message (13.1): its cells wrap long words, and one that
  // still cannot fit scrolls sideways inside its box (20.1).
  table: ({ children }) => (
    <div className="md-table">
      <table>{children}</table>
    </div>
  ),
  td: ({ node, ...props }) => <td {...props} className={oneWord(node)} />,
  th: ({ node, ...props }) => <th {...props} className={oneWord(node)} />,
};

/** A cell of one word (a date, a state, a path) never breaks: `-` and `/` would (20.1). */
function oneWord(node: Element | undefined): string | undefined {
  const text = node ? shown(node).trim() : "";
  return text && !/\s/.test(text) ? "md-word" : undefined;
}

/**
 * Markdown text (8.6, spike Q5), such as a pull request's: GFM (tables, strikethrough, task
 * lists), raw HTML only through the `SCHEMA` allowlist (D20.2), links filtered by `safeUrl`. Memoized on the text, so an unchanged one is not parsed again.
 */
export const Markdown = memo(function Markdown({
  text,
  document = false,
}: {
  text: string;
  /** A whole file in its tab (14.6): the document style, with coloured code blocks. */
  document?: boolean;
}) {
  return (
    <div className={document ? "markdown markdown-doc" : "markdown"}>
      <ReactMarkdown
        remarkPlugins={PLUGINS}
        rehypePlugins={REHYPE}
        urlTransform={safeUrl}
        components={document ? { ...COMPONENTS, ...DOC_COMPONENTS } : COMPONENTS}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});
