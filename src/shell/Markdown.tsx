import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { setNotice } from "../store";

/** Only these links open (8.6): anything else (`javascript:`, `file:`, relative) is plain text. */
export const safeUrl = (url: string) => (/^(https?:|mailto:)/i.test(url.trim()) ? url : "");

/**
 * A link opens outside the app, in the default browser or mail app (the opener plugin); the
 * webview never navigates. Outside Tauri (browser, mock transport) nothing opens and the status
 * bar says so.
 */
export async function openLink(url: string, tauri = isTauri()): Promise<void> {
  if (!tauri) return setNotice(`Only the Hive app opens links: ${url}`);
  try {
    await openUrl(url);
  } catch (error) {
    setNotice(String(error));
  }
}

const PLUGINS = [remarkGfm];
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
      >
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  // Never load an image (untrusted text, no network): its alt text stands in.
  img: ({ alt }) => (alt ? <span className="md-img">{alt}</span> : null),
  // Wide tables scroll sideways inside the message.
  table: ({ children }) => (
    <div className="md-table">
      <table>{children}</table>
    </div>
  ),
};

/**
 * Claude's text as Markdown (8.6, spike Q5): GFM (tables, strikethrough, task lists), no raw
 * HTML (react-markdown shows it as text without `rehype-raw`), links filtered by `safeUrl`.
 * Memoized on the text: while a live entry grows (7.3h) only it is parsed again.
 */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown remarkPlugins={PLUGINS} urlTransform={safeUrl} components={COMPONENTS}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
